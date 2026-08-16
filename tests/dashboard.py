#!/usr/bin/env python3
"""Exercise owner authentication, CSP assets, CSRF, and emergency controls."""

from __future__ import annotations

import base64
import http.client
import json
import os
import signal
import socket
import subprocess
import sys
import tempfile
import time
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
BINARY = ROOT / "target" / "debug" / "treasury"


def run(*args: str) -> dict:
    result = subprocess.run(
        [str(BINARY), *args],
        cwd=ROOT,
        check=True,
        capture_output=True,
        text=True,
    )
    return json.loads(result.stdout)


def request(
    port: int,
    method: str,
    path: str,
    headers: dict[str, str] | None = None,
    body: bytes | None = None,
) -> tuple[int, list[tuple[str, str]], bytes]:
    connection = http.client.HTTPConnection("127.0.0.1", port, timeout=5)
    connection.request(method, path, body=body, headers=headers or {})
    response = connection.getresponse()
    result = (response.status, response.getheaders(), response.read())
    connection.close()
    return result


with tempfile.TemporaryDirectory(prefix="agent-treasury-dashboard-") as raw_directory:
    directory = Path(raw_directory)
    owner_file = directory / "owner.token"
    access_file = directory / "dashboard.token"
    access_token = "synthetic-dashboard-owner-secret"
    access_file.write_text(access_token + "\n", encoding="utf-8")
    os.chmod(access_file, 0o600)
    run(
        "init",
        "--data-dir",
        str(directory),
        "--owner-token-file",
        str(owner_file),
        "--balance-minor",
        "10000",
    )
    same_credential = subprocess.run(
        [
            sys.executable,
            str(ROOT / "apps" / "owner-dashboard" / "server.py"),
            "--socket-path",
            str(directory / "unused.sock"),
            "--owner-token-file",
            str(owner_file),
            "--access-token-file",
            str(owner_file),
        ],
        cwd=ROOT,
        capture_output=True,
        text=True,
    )
    assert same_credential.returncode != 0
    assert "must differ" in same_credential.stderr
    socket_path = directory / "treasury.sock"
    owner_socket_path = directory / "owner.sock"
    daemon = subprocess.Popen(
        [str(BINARY), "serve", "--data-dir", str(directory), "--socket", str(socket_path)],
        cwd=ROOT,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
    )
    dashboard: subprocess.Popen[str] | None = None
    try:
        for _ in range(100):
            if socket_path.exists() and owner_socket_path.exists():
                break
            time.sleep(0.05)
        with socket.socket() as probe:
            probe.bind(("127.0.0.1", 0))
            port = probe.getsockname()[1]
        dashboard = subprocess.Popen(
            [
                "python3",
                str(ROOT / "apps" / "owner-dashboard" / "server.py"),
                "--socket-path",
                str(owner_socket_path),
                "--owner-token-file",
                str(owner_file),
                "--access-token-file",
                str(access_file),
                "--port",
                str(port),
            ],
            cwd=ROOT,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
        )
        for _ in range(100):
            try:
                status, _, _ = request(port, "GET", "/")
                if status == 401:
                    break
            except OSError:
                time.sleep(0.05)
        else:
            raise SystemExit("dashboard did not start with owner authentication")

        status, _, _ = request(port, "GET", "/")
        assert status == 401
        authorization = "Basic " + base64.b64encode(
            f"owner:{access_token}".encode("utf-8")
        ).decode("ascii")
        status, headers, body = request(
            port,
            "GET",
            "/",
            {"Authorization": authorization},
        )
        assert status == 200 and b"Emergency stop" in body
        cookies = [value for name, value in headers if name.lower() == "set-cookie"]
        cookie_header = "; ".join(value.split(";", 1)[0] for value in cookies)
        csrf = next(
            value.split("=", 1)[1]
            for value in cookie_header.split("; ")
            if value.startswith("csrf=")
        )
        authenticated = {"Authorization": authorization, "Cookie": cookie_header}
        assert request(port, "GET", "/app.js", authenticated)[0] == 200
        assert request(port, "GET", "/style.css", authenticated)[0] == 200
        assert request(port, "GET", "/api/status", authenticated)[0] == 200
        unauthenticated_attack = {
            "Origin": f"http://127.0.0.1:{port}",
            "Cookie": cookie_header,
            "X-CSRF-Token": csrf,
            "Content-Type": "application/json",
        }
        assert request(
            port,
            "POST",
            "/api/emergency-stop",
            unauthenticated_attack,
            b'{"stopped":false}',
        )[0] == 401
        owner_headers = dict(unauthenticated_attack, Authorization=authorization)
        status, _, body = request(
            port,
            "POST",
            "/api/emergency-stop",
            owner_headers,
            b'{"stopped":true}',
        )
        assert status == 200 and json.loads(body)["emergency_stop"] is True
        print("owner dashboard authentication and emergency-control assertions passed")
    finally:
        if dashboard is not None:
            dashboard.send_signal(signal.SIGTERM)
            dashboard.wait(timeout=5)
        daemon.send_signal(signal.SIGTERM)
        daemon.wait(timeout=5)
