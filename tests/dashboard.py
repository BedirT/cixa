#!/usr/bin/env python3
"""Exercise owner authentication, CSP assets, CSRF, and emergency controls."""

from __future__ import annotations

import base64
import http.client
import json
import os
import signal
import socket
import stat
import subprocess
import sys
import tempfile
import time
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
BINARY = ROOT / "target" / "debug" / "cixa"


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


def rpc(socket_path: Path, token: str, operation: dict) -> dict:
    envelope = {
        "api_version": "v1",
        "request_id": "dashboard-integration",
        "token": token,
        "operation": operation,
    }
    with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as channel:
        channel.connect(str(socket_path))
        channel.sendall((json.dumps(envelope, separators=(",", ":")) + "\n").encode())
        response = b""
        while b"\n" not in response:
            response += channel.recv(65536)
    decoded = json.loads(response.split(b"\n", 1)[0])
    if not decoded["ok"]:
        raise RuntimeError(decoded["error"])
    return decoded["data"]


with tempfile.TemporaryDirectory(prefix="cixa-dashboard-") as raw_directory:
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
    socket_path = directory / "cixa.sock"
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
        assert status == 200 and b"Stop all spending" in body and b"Cixa" in body
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
        mark_status, mark_headers, mark_body = request(
            port, "GET", "/cixa-mark.svg", authenticated
        )
        assert mark_status == 200 and b"<svg" in mark_body
        assert dict(mark_headers)["Content-Type"] == "image/svg+xml"
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
        assert request(
            port,
            "GET",
            "/api/overview",
            dict(authenticated, Host="attacker.example"),
        )[0] == 400
        assert request(
            port,
            "POST",
            "/api/emergency-stop",
            dict(owner_headers, Origin="http://attacker.example"),
            b'{"stopped":true}',
        )[0] == 403
        assert request(
            port,
            "POST",
            "/api/emergency-stop",
            dict(owner_headers, **{"X-CSRF-Token": "wrong"}),
            b'{"stopped":true}',
        )[0] == 403
        assert request(
            port,
            "POST",
            "/api/emergency-stop",
            owner_headers,
            b'{"stopped":true,"extra":false}',
        )[0] == 400
        assert request(
            port,
            "POST",
            "/api/emergency-stop",
            owner_headers,
            b" " * (32 * 1024 + 1),
        )[0] == 400

        def owner_post(path: str, value: dict) -> dict:
            status, _, response_body = request(
                port,
                "POST",
                path,
                owner_headers,
                json.dumps(value).encode(),
            )
            assert status == 200, (path, status, response_body)
            return json.loads(response_body)

        status, _, body = request(
            port,
            "POST",
            "/api/emergency-stop",
            owner_headers,
            b'{"stopped":true}',
        )
        assert status == 200 and json.loads(body)["emergency_stop"] is True
        owner_post("/api/emergency-stop", {"stopped": False})

        status, _, body = request(port, "GET", "/api/overview", authenticated)
        overview = json.loads(body)
        assert status == 200 and overview["provider"]["balance_status"] == "simulated_provider_verified", (
            status,
            overview,
        )
        policy = next(iter(overview["policies"].values()))
        created = owner_post(
            "/api/agents/create",
            {
                "name": "dashboard-agent",
                "token_filename": "dashboard-agent.token",
                "policy": policy,
                "mode": "bounded_autonomous",
                "ttl_secs": 3600,
            },
        )
        agent_id = created["agent_id"]
        assert "capability_token" not in created
        token_path = directory / "agent-tokens" / "dashboard-agent.token"
        assert created["agent_token_file"] == str(token_path)
        assert stat.S_IMODE(token_path.stat().st_mode) == 0o600
        agent_token = token_path.read_text(encoding="utf-8").strip()
        assert agent_token not in json.dumps(json.loads(request(port, "GET", "/api/overview", authenticated)[2]))
        policy["max_per_transaction"] = {"minor": 2000, "currency": "CAD"}
        assert owner_post("/api/policies/update", {"agent_id": agent_id, "policy": policy})["policy"]["version"] == 2
        owner_post("/api/agents/mode", {"agent_id": agent_id, "mode": "bounded_autonomous"})
        owner_post("/api/agents/arm-session", {"agent_id": agent_id, "ttl_secs": 600})
        owner_post(
            "/api/merchants/approve",
            {"agent_id": agent_id, "merchant_domain": "merchant.example.test"},
        )
        owner_post(
            "/api/receive",
            {
                "method": "interac_e_transfer",
                "address": "public-inbox@example.invalid",
                "memo_template": "AGENT-{agent_id}-{intent_id}",
            },
        )
        owner_post(
            "/api/provider/manual",
            {
                "credential_reference": "keychain://cixa/dashboard-card",
                "provider_kind": "os-credential-store",
                "last_four": "1111",
                "balance": {"minor": 10000, "currency": "CAD"},
                "balance_status": "owner_confirmed",
                "balance_ttl_secs": 900,
            },
        )
        purchase = {
            "idempotency_key": "dashboard-purchase",
            "amount": {"minor": 500, "currency": "CAD"},
            "final_total": {"minor": 500, "currency": "CAD"},
            "merchant_domain": "merchant.example.test",
            "category": "software",
            "items": [{"label": "dashboard item", "quantity": 1, "unit_price_minor": 500}],
            "recurring": False,
            "trial_auto_renew": False,
            "stored_card": False,
            "tip_minor": 0,
            "preauthorization": False,
            "installments": False,
            "fulfillment_profile": "digital-email",
            "payment_form": "hosted_fields",
            "redirect_chain": ["https://merchant.example.test/checkout"],
            "attempts": 1,
            "session_id": "dashboard-session",
            "scenario": "normal",
        }
        intent = rpc(socket_path, agent_token, {"type": "create_purchase_intent", "request": purchase})
        assert intent["state"] == "approval_required"
        pending = json.loads(request(port, "GET", "/api/overview", authenticated)[2])
        assert pending["pending_approvals"][0]["id"] == intent["id"]
        assert pending["pending_approvals"][0]["checkout_facts"] == {
            "payment_form": "hosted_fields",
            "redirect_chain": ["https://merchant.example.test/checkout"],
            "recurring": False,
            "trial_auto_renew": False,
            "stored_card": False,
            "tip_minor": 0,
            "preauthorization": False,
            "installments": False,
            "scenario": "normal",
        }
        detail_status, _, detail_body = request(
            port, "GET", f"/api/intents/{intent['id']}", authenticated
        )
        assert detail_status == 200 and json.loads(detail_body)["id"] == intent["id"]
        assert request(port, "GET", f"/api/receipts/{intent['id']}", authenticated)[0] == 404
        assert request(port, "GET", "/api/intents/not%2Fsafe", authenticated)[0] == 400
        denied = owner_post("/api/approvals/deny", {"intent_id": intent["id"]})
        assert denied["state"] == "cancelled" and denied["last_error"] == "owner_denied"
        assert request(
            port,
            "POST",
            "/api/approvals/deny",
            owner_headers,
            json.dumps({"intent_id": intent["id"]}).encode(),
        )[0] == 400

        purchase["idempotency_key"] = "dashboard-purchase-handoff"
        intent = rpc(
            socket_path,
            agent_token,
            {"type": "create_purchase_intent", "request": purchase},
        )
        assert intent["state"] == "approval_required"
        owner_post("/api/approvals/approve", {"intent_id": intent["id"]})
        handoff = owner_post("/api/handoff/begin", {"intent_id": intent["id"]})
        assert handoff["status"] == "owner_handoff_ready"
        execution = owner_post("/api/handoff/complete", {"intent_id": intent["id"]})
        assert execution["status"] == "unknown"
        owner_post(
            "/api/reconcile",
            {"intent_id": intent["id"], "outcome": "settled", "provider_reference": "dashboard-ref-1"},
        )
        receipt_status, _, receipt_body = request(
            port, "GET", f"/api/receipts/{intent['id']}", authenticated
        )
        assert receipt_status == 200
        assert json.loads(receipt_body)["personal_information_redacted"] is True
        owner_post(
            "/api/deposits/record",
            {
                "amount": {"minor": 100, "currency": "CAD"},
                "source": "dashboard-notification",
                "verified": False,
                "agent_id": None,
                "external_reference": "dashboard-deposit-1",
            },
        )
        assert request(port, "GET", "/api/transactions", authenticated)[0] == 200
        assert request(port, "GET", "/api/audit", authenticated)[0] == 200
        export_status, _, export_body = request(port, "GET", "/api/export", authenticated)
        assert export_status == 200 and json.loads(export_body)["sanitized"] is True
        owner_post("/api/agents/revoke", {"agent_id": agent_id})
        assert rpc(owner_socket_path, owner_file.read_text().strip(), {"type": "owner_get_dashboard"})["agents"][0]["revoked"] is True
        print("owner dashboard full-workflow assertions passed")
    finally:
        if dashboard is not None:
            dashboard.send_signal(signal.SIGTERM)
            dashboard.wait(timeout=5)
        daemon.send_signal(signal.SIGTERM)
        daemon.wait(timeout=5)
