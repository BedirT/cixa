#!/usr/bin/env python3
"""Loopback-only owner dashboard bridge.

This intentionally uses the standard library. It never serves the agent token,
card secrets, audit files, or a remote asset. Mutations are POST-only and require
an Origin check plus a synchronizer token.
"""

from __future__ import annotations

import argparse
import base64
import http.server
import json
import os
import secrets
import socket
import stat
from pathlib import Path
from typing import Any


MAX_BODY = 32 * 1024


def read_private_token(path_value: str) -> str:
    path = Path(path_value)
    metadata = path.lstat()
    if stat.S_ISLNK(metadata.st_mode) or not stat.S_ISREG(metadata.st_mode):
        raise ValueError(f"{path} must be a regular file")
    if metadata.st_mode & 0o077:
        raise ValueError(f"{path} permissions are too broad")
    token = path.read_text(encoding="utf-8").strip()
    if not token:
        raise ValueError(f"{path} is empty")
    return token


class DashboardState:
    def __init__(
        self,
        socket_path: str,
        owner_token_file: str,
        access_token_file: str,
        port: int,
        agent_token_directory: str | None = None,
    ) -> None:
        self.socket_path = socket_path
        self.owner_token = read_private_token(owner_token_file)
        self.access_token = read_private_token(access_token_file)
        if secrets.compare_digest(self.owner_token, self.access_token):
            raise ValueError("dashboard access credential must differ from the broker owner credential")
        self.csrf = secrets.token_urlsafe(32)
        self.session = secrets.token_urlsafe(32)
        self.port = port
        self.agent_token_directory = Path(
            agent_token_directory or Path(owner_token_file).parent / "agent-tokens"
        )
        self._prepare_agent_token_directory()

    def _prepare_agent_token_directory(self) -> None:
        self.agent_token_directory.mkdir(mode=0o700, parents=True, exist_ok=True)
        metadata = self.agent_token_directory.lstat()
        if stat.S_ISLNK(metadata.st_mode) or not stat.S_ISDIR(metadata.st_mode):
            raise ValueError("agent token directory must be a directory, not a symlink")
        if metadata.st_mode & 0o077:
            raise ValueError("agent token directory permissions are too broad")

    def create_agent(self, operation: dict[str, Any], token_filename: Any) -> Any:
        if not isinstance(token_filename, str) or not 1 <= len(token_filename) <= 64:
            raise ValueError("token_filename must be a short string")
        if token_filename in {".", ".."} or not all(
            character.isalnum() or character in "._-" for character in token_filename
        ):
            raise ValueError("token_filename contains unsupported characters")
        self._prepare_agent_token_directory()
        token_path = self.agent_token_directory / token_filename
        flags = os.O_WRONLY | os.O_CREAT | os.O_EXCL
        if hasattr(os, "O_NOFOLLOW"):
            flags |= os.O_NOFOLLOW
        descriptor = os.open(token_path, flags, 0o600)
        try:
            value = self.call(operation)
            token = value.pop("capability_token", None)
            if not isinstance(token, str) or not token:
                raise RuntimeError("broker did not return an agent capability")
            os.write(descriptor, (token + "\n").encode("utf-8"))
            os.fsync(descriptor)
        except BaseException:
            os.close(descriptor)
            token_path.unlink(missing_ok=True)
            raise
        os.close(descriptor)
        value["agent_token_file"] = str(token_path)
        return value

    def call(self, operation: dict[str, Any]) -> Any:
        request = {
            "api_version": "v1",
            "request_id": secrets.token_hex(16),
            "token": self.owner_token,
            "operation": operation,
        }
        with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as channel:
            channel.settimeout(10)
            channel.connect(self.socket_path)
            channel.sendall((json.dumps(request, separators=(",", ":")) + "\n").encode("utf-8"))
            response = b""
            while b"\n" not in response:
                chunk = channel.recv(64 * 1024)
                if not chunk:
                    raise RuntimeError("broker closed the dashboard connection")
                response += chunk
                if len(response) > 256 * 1024:
                    raise RuntimeError("broker response is too large")
        decoded = json.loads(response.split(b"\n", 1)[0].decode("utf-8"))
        if not decoded.get("ok"):
            raise RuntimeError(decoded.get("error", "broker rejected dashboard request"))
        return decoded.get("data")


def make_handler(state: DashboardState):
    class Handler(http.server.BaseHTTPRequestHandler):
        server_version = "cixa-dashboard/0.1"

        def log_message(self, format: str, *args: object) -> None:
            print(format % args)

        def _headers(self, content_type: str) -> None:
            self.send_header("Content-Type", content_type)
            self.send_header("Cache-Control", "no-store")
            self.send_header("Content-Security-Policy", "default-src 'self'; script-src 'self'; style-src 'self'; object-src 'none'; base-uri 'none'; frame-ancestors 'none'")
            self.send_header("Referrer-Policy", "no-referrer")
            self.send_header("X-Content-Type-Options", "nosniff")
            self.send_header("X-Frame-Options", "DENY")

        def _host_allowed(self) -> bool:
            host = self.headers.get("Host", "").split(":", 1)[0]
            return host in {"127.0.0.1", "localhost", "::1"}

        def _send_json(self, status: int, value: Any) -> None:
            body = json.dumps(value, separators=(",", ":")).encode("utf-8")
            self.send_response(status)
            self._headers("application/json; charset=utf-8")
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)

        def _same_origin(self) -> bool:
            origin = self.headers.get("Origin", "")
            return origin in {f"http://127.0.0.1:{state.port}", f"http://localhost:{state.port}"}

        def _authorized(self) -> bool:
            header = self.headers.get("Authorization", "")
            if not header.startswith("Basic "):
                return False
            try:
                decoded = base64.b64decode(header[6:], validate=True).decode("utf-8")
                username, password = decoded.split(":", 1)
            except (ValueError, UnicodeDecodeError):
                return False
            return username == "owner" and secrets.compare_digest(password, state.access_token)

        def _session_ok(self) -> bool:
            cookie = self.headers.get("Cookie", "")
            session_cookie = next(
                (part.split("=", 1)[1] for part in cookie.split("; ") if part.startswith("session=")),
                "",
            )
            return secrets.compare_digest(session_cookie, state.session)

        def _require_owner(self, require_session: bool = True) -> bool:
            if self._authorized() and (not require_session or self._session_ok()):
                return True
            self.send_response(401)
            self._headers("application/json; charset=utf-8")
            self.send_header("WWW-Authenticate", 'Basic realm="cixa owner"')
            body = b'{"error":"owner authentication required"}'
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)
            return False

        def _csrf_ok(self) -> bool:
            cookie = self.headers.get("Cookie", "")
            csrf_cookie = next((part.split("=", 1)[1] for part in cookie.split("; ") if part.startswith("csrf=")), "")
            return secrets.compare_digest(csrf_cookie, state.csrf) and secrets.compare_digest(self.headers.get("X-CSRF-Token", ""), state.csrf)

        def _read_json(self) -> dict[str, Any]:
            length = int(self.headers.get("Content-Length", "0"))
            if length <= 0 or length > MAX_BODY:
                raise ValueError("invalid request body length")
            value = json.loads(self.rfile.read(length).decode("utf-8"))
            if not isinstance(value, dict):
                raise ValueError("request must be an object")
            return value

        def _owner_operation(self, path: str, body: dict[str, Any]) -> dict[str, Any]:
            schemas: dict[str, tuple[str, set[str]]] = {
                "/api/emergency-stop": ("owner_set_emergency_stop", {"stopped"}),
                "/api/agents/create": (
                    "owner_create_agent",
                    {"name", "policy", "mode", "ttl_secs", "token_filename"},
                ),
                "/api/agents/revoke": ("owner_revoke_agent", {"agent_id"}),
                "/api/agents/mode": ("owner_set_agent_mode", {"agent_id", "mode"}),
                "/api/agents/arm-session": ("owner_arm_agent_session", {"agent_id", "ttl_secs"}),
                "/api/policies/update": ("owner_update_policy", {"agent_id", "policy"}),
                "/api/approvals/approve": ("owner_approve_intent", {"intent_id"}),
                "/api/merchants/approve": ("owner_approve_merchant", {"agent_id", "merchant_domain"}),
                "/api/reconcile": ("owner_reconcile", {"intent_id", "outcome", "provider_reference"}),
                "/api/provider/manual": (
                    "owner_configure_manual_provider",
                    {"credential_reference", "provider_kind", "last_four", "balance", "balance_status", "balance_ttl_secs"},
                ),
                "/api/receive": ("owner_configure_receive_instructions", {"method", "address", "memo_template"}),
                "/api/deposits/record": (
                    "owner_record_deposit",
                    {"amount", "source", "verified", "agent_id", "external_reference"},
                ),
            }
            if path not in schemas:
                raise KeyError(path)
            operation_type, fields = schemas[path]
            if set(body) != fields:
                raise ValueError(f"body fields must be exactly {sorted(fields)}")
            return {"type": operation_type, **body}

        def do_GET(self) -> None:  # noqa: N802
            if not self._host_allowed():
                self._send_json(400, {"error": "untrusted Host"})
                return
            if self.path == "/":
                if not self._require_owner(require_session=False):
                    return
                body = Path(__file__).with_name("index.html").read_bytes()
                self.send_response(200)
                self._headers("text/html; charset=utf-8")
                self.send_header("Set-Cookie", f"csrf={state.csrf}; Path=/; SameSite=Strict")
                self.send_header(
                    "Set-Cookie",
                    f"session={state.session}; Path=/; HttpOnly; SameSite=Strict",
                )
                self.send_header("Content-Length", str(len(body)))
                self.end_headers()
                self.wfile.write(body)
                return
            if self.path in {"/app.js", "/style.css"}:
                if not self._require_owner():
                    return
                name = self.path.removeprefix("/")
                body = Path(__file__).with_name(name).read_bytes()
                content_type = (
                    "application/javascript; charset=utf-8"
                    if name.endswith(".js")
                    else "text/css; charset=utf-8"
                )
                self.send_response(200)
                self._headers(content_type)
                self.send_header("Content-Length", str(len(body)))
                self.end_headers()
                self.wfile.write(body)
                return
            if self.path == "/api/status":
                if not self._require_owner():
                    return
                try:
                    self._send_json(200, state.call({"type": "get_status"}))
                except (OSError, RuntimeError, ValueError):
                    self._send_json(502, {"error": "broker request failed"})
                return
            if self.path in {"/api/overview", "/api/transactions", "/api/audit", "/api/export"}:
                if not self._require_owner():
                    return
                try:
                    if self.path == "/api/overview":
                        value = state.call({"type": "owner_get_dashboard"})
                    elif self.path == "/api/transactions":
                        value = state.call({"type": "list_transactions"})
                    elif self.path == "/api/audit":
                        value = state.call({"type": "owner_list_audit"})
                    else:
                        value = {
                            "overview": state.call({"type": "owner_get_dashboard"}),
                            "audit": state.call({"type": "owner_list_audit"}),
                            "sanitized": True,
                        }
                    self._send_json(200, value)
                except (OSError, RuntimeError, ValueError):
                    self._send_json(502, {"error": "broker request failed"})
                return
            self._send_json(404, {"error": "not found"})

        def do_POST(self) -> None:  # noqa: N802
            if not self._host_allowed() or not self._require_owner():
                return
            if not self._same_origin() or not self._csrf_ok():
                self._send_json(403, {"error": "origin or CSRF validation failed"})
                return
            try:
                body = self._read_json()
                operation = self._owner_operation(self.path, body)
                if operation["type"] == "owner_set_emergency_stop" and not isinstance(
                    operation["stopped"], bool
                ):
                    raise ValueError("stopped must be boolean")
                if operation["type"] == "owner_create_agent":
                    token_filename = operation.pop("token_filename")
                    value = state.create_agent(operation, token_filename)
                else:
                    value = state.call(operation)
                self._send_json(200, value)
            except KeyError:
                self._send_json(404, {"error": "not found"})
            except (OSError, RuntimeError, ValueError, json.JSONDecodeError):
                self._send_json(400, {"error": "request rejected"})

    return Handler


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--socket-path", required=True)
    parser.add_argument("--owner-token-file", required=True)
    parser.add_argument("--access-token-file", required=True)
    parser.add_argument("--agent-token-directory")
    parser.add_argument("--port", type=int, default=8765)
    args = parser.parse_args()
    if not 1024 <= args.port <= 65535:
        raise SystemExit("port must be between 1024 and 65535")
    state = DashboardState(
        args.socket_path,
        args.owner_token_file,
        args.access_token_file,
        args.port,
        args.agent_token_directory,
    )
    server = http.server.ThreadingHTTPServer(("127.0.0.1", args.port), make_handler(state))
    print(f"owner dashboard listening on http://127.0.0.1:{args.port}")
    server.serve_forever()


if __name__ == "__main__":
    main()
