#!/usr/bin/env python3
"""Loopback-only owner dashboard bridge.

This intentionally uses the standard library. It never serves the agent token,
card secrets, audit files, or a remote asset. Mutations are POST-only and require
an Origin check plus a synchronizer token.
"""

from __future__ import annotations

import argparse
import http.server
import json
import os
import secrets
import socket
import stat
from pathlib import Path
from typing import Any
from urllib.parse import parse_qs, urlsplit


MAX_BODY = 32 * 1024


class ActivationUncertain(RuntimeError):
    def __init__(self, token_path: Path) -> None:
        self.token_path = token_path
        super().__init__(
            "Activation may have completed. The prepared token was retained at "
            f"{token_path}. Refresh Agents and reconcile that file before retrying."
        )


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

    def _sync_agent_token_directory(self) -> None:
        descriptor = os.open(self.agent_token_directory, os.O_RDONLY)
        try:
            os.fsync(descriptor)
        finally:
            os.close(descriptor)

    def write_capability(self, operation: dict[str, Any], token_filename: Any) -> Any:
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
        token = secrets.token_hex(32)
        payload = (token + "\n").encode("ascii")
        activation_started = False
        try:
            written = 0
            while written < len(payload):
                count = os.write(descriptor, payload[written:])
                if count <= 0:
                    raise OSError("agent capability write made no progress")
                written += count
            os.fsync(descriptor)
            os.close(descriptor)
            descriptor = -1
            self._sync_agent_token_directory()
            operation["capability_token"] = token
            activation_started = True
            value = self.call(operation)
        except BaseException as error:
            if descriptor >= 0:
                os.close(descriptor)
            if not activation_started:
                token_path.unlink(missing_ok=True)
                self._sync_agent_token_directory()
                raise
            raise ActivationUncertain(token_path) from error
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

        def _session_ok(self) -> bool:
            cookie = self.headers.get("Cookie", "")
            session_cookie = next(
                (part.split("=", 1)[1] for part in cookie.split("; ") if part.startswith("session=")),
                "",
            )
            return secrets.compare_digest(session_cookie, state.session)

        def _require_owner(self) -> bool:
            if self._session_ok():
                return True
            self._send_json(401, {"error": "owner session required"})
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
                    "owner_create_agent_prepared",
                    {"name", "policy", "mode", "ttl_secs", "token_filename"},
                ),
                "/api/agents/revoke": ("owner_revoke_agent", {"agent_id"}),
                "/api/agents/rotate": (
                    "owner_rotate_agent_capability",
                    {"agent_id", "ttl_secs", "token_filename"},
                ),
                "/api/agents/mode": ("owner_set_agent_mode", {"agent_id", "mode"}),
                "/api/agents/arm-session": ("owner_arm_agent_session", {"agent_id", "ttl_secs"}),
                "/api/policies/update": ("owner_update_policy", {"agent_id", "policy"}),
                "/api/approvals/approve": ("owner_approve_intent", {"intent_id"}),
                "/api/approvals/deny": ("owner_deny_intent", {"intent_id"}),
                "/api/handoff/begin": ("owner_begin_manual_handoff", {"intent_id"}),
                "/api/handoff/complete": ("owner_complete_manual_handoff", {"intent_id"}),
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
                body = Path(__file__).with_name("index.html").read_bytes()
                self.send_response(200)
                self._headers("text/html; charset=utf-8")
                self.send_header("Content-Length", str(len(body)))
                self.end_headers()
                self.wfile.write(body)
                return
            static_files = {
                "/app.js": (
                    Path(__file__).with_name("app.js"),
                    "application/javascript; charset=utf-8",
                ),
                "/style.css": (
                    Path(__file__).with_name("style.css"),
                    "text/css; charset=utf-8",
                ),
                "/cixa-mark.svg": (Path(__file__).with_name("cixa-mark.svg"), "image/svg+xml"),
                "/assets/manrope-latin.woff2": (
                    Path(__file__).with_name("assets") / "manrope-latin.woff2",
                    "font/woff2",
                ),
                "/assets/newsreader-latin.woff2": (
                    Path(__file__).with_name("assets") / "newsreader-latin.woff2",
                    "font/woff2",
                ),
            }
            if self.path in static_files:
                static_path, content_type = static_files[self.path]
                body = static_path.read_bytes()
                self.send_response(200)
                self._headers(content_type)
                self.send_header("Content-Length", str(len(body)))
                self.end_headers()
                self.wfile.write(body)
                return
            parsed = urlsplit(self.path)
            path = parsed.path
            if path.startswith("/api/intents/") or path.startswith("/api/receipts/"):
                if not self._require_owner():
                    return
                intent_id = path.rsplit("/", 1)[-1]
                if not 1 <= len(intent_id) <= 128 or not all(
                    character.isalnum() or character in "_-" for character in intent_id
                ):
                    self._send_json(400, {"error": "invalid intent identifier"})
                    return
                operation_type = (
                    "get_purchase_intent"
                    if path.startswith("/api/intents/")
                    else "get_receipt"
                )
                try:
                    self._send_json(
                        200,
                        state.call({"type": operation_type, "intent_id": intent_id}),
                    )
                except (OSError, RuntimeError, ValueError):
                    self._send_json(404, {"error": "intent detail is unavailable"})
                return
            if path == "/api/status":
                if not self._require_owner():
                    return
                try:
                    self._send_json(200, state.call({"type": "get_status"}))
                except (OSError, RuntimeError, ValueError):
                    self._send_json(502, {"error": "broker request failed"})
                return
            if path in {"/api/overview", "/api/transactions", "/api/audit", "/api/export"}:
                if not self._require_owner():
                    return
                try:
                    if path == "/api/overview":
                        value = state.call({"type": "owner_get_dashboard"})
                    elif path in {"/api/transactions", "/api/audit"}:
                        query = parse_qs(parsed.query, strict_parsing=True)
                        if set(query) - {"cursor", "limit"} or any(len(values) != 1 for values in query.values()):
                            raise ValueError("invalid page query")
                        limit = int(query.get("limit", ["25"])[0])
                        if not 1 <= limit <= 50:
                            raise ValueError("invalid page limit")
                        raw_cursor = query.get("cursor", [None])[0]
                        if path == "/api/transactions" and raw_cursor is not None and (
                            not 1 <= len(raw_cursor) <= 128
                            or not all(character.isalnum() or character in "_-" for character in raw_cursor)
                        ):
                            raise ValueError("invalid transaction cursor")
                        operation = {
                            "type": "owner_list_transactions_page" if path == "/api/transactions" else "owner_list_audit_page",
                            "cursor": int(raw_cursor) if path == "/api/audit" and raw_cursor is not None else raw_cursor,
                            "limit": limit,
                        }
                        value = state.call(operation)
                    else:
                        value = {
                            "overview": state.call({"type": "owner_get_dashboard"}),
                            "audit": state.call({"type": "owner_list_audit_recent"}),
                            "sanitized": True,
                        }
                    self._send_json(200, value)
                except ValueError:
                    self._send_json(400, {"error": "invalid page request"})
                except (OSError, RuntimeError):
                    self._send_json(502, {"error": "broker request failed"})
                return
            self._send_json(404, {"error": "not found"})

        def do_POST(self) -> None:  # noqa: N802
            if not self._host_allowed():
                return
            if self.path == "/api/session":
                if not self._same_origin():
                    self._send_json(403, {"error": "origin validation failed"})
                    return
                try:
                    body = self._read_json()
                    if set(body) != {"access_token"} or not isinstance(body["access_token"], str):
                        raise ValueError("invalid session request")
                    if not secrets.compare_digest(body["access_token"], state.access_token):
                        self._send_json(401, {"error": "access credential rejected"})
                        return
                    response = b'{"ok":true}'
                    self.send_response(200)
                    self._headers("application/json; charset=utf-8")
                    self.send_header("Set-Cookie", f"csrf={state.csrf}; Path=/; SameSite=Strict")
                    self.send_header("Set-Cookie", f"session={state.session}; Path=/; HttpOnly; SameSite=Strict")
                    self.send_header("Content-Length", str(len(response)))
                    self.end_headers()
                    self.wfile.write(response)
                except (ValueError, json.JSONDecodeError):
                    self._send_json(400, {"error": "request rejected"})
                return
            if not self._require_owner():
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
                if operation["type"] in {
                    "owner_create_agent_prepared",
                    "owner_rotate_agent_capability",
                }:
                    token_filename = operation.pop("token_filename")
                    value = state.write_capability(operation, token_filename)
                else:
                    value = state.call(operation)
                self._send_json(200, value)
            except KeyError:
                self._send_json(404, {"error": "not found"})
            except ActivationUncertain as error:
                self._send_json(
                    409,
                    {
                        "error": str(error),
                        "activation_uncertain": True,
                        "agent_token_file": str(error.token_path),
                    },
                )
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
