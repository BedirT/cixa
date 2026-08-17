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
import re
import secrets
import subprocess
import socket
import stat
import threading
import time
from pathlib import Path
from typing import Any
from urllib.parse import parse_qs, urlsplit


MAX_BODY = 32 * 1024
PROFILE_ID = re.compile(r"^[a-z0-9][a-z0-9_-]{0,63}$")
DOMAIN = re.compile(r"^(?=.{1,253}$)(?:[a-z0-9](?:[a-z0-9-]{0,61}[a-z0-9])?\.)+[a-z]{2,63}$")


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
        cixa_binary: str | None = None,
        checkout_runtime_directory: str | None = None,
        checkout_profiles_directory: str | None = None,
        checkout_browser_executable: str | None = None,
        agent_gid: int | None = None,
    ) -> None:
        self.socket_path = socket_path
        self.owner_token = read_private_token(owner_token_file)
        self.access_token = read_private_token(access_token_file)
        if secrets.compare_digest(self.owner_token, self.access_token):
            raise ValueError("dashboard access credential must differ from the broker owner credential")
        self.csrf = secrets.token_urlsafe(32)
        self.session = secrets.token_urlsafe(32)
        self.port = port
        self.agent_gid = agent_gid
        if self.agent_gid is not None and (self.agent_gid < 1 or self.agent_gid == os.getegid()):
            raise ValueError("agent GID must be positive and differ from the owner primary group")
        self.agent_token_directory = Path(
            agent_token_directory or Path(owner_token_file).parent / "agent-tokens"
        ).expanduser().resolve()
        self._prepare_agent_token_directory()
        project_binary = Path(__file__).resolve().parents[2] / "target" / "debug" / "cixa"
        self.cixa_binary = (Path(cixa_binary) if cixa_binary else project_binary).expanduser().resolve()
        base = Path(owner_token_file).parent
        self.checkout_runtime_directory = Path(
            checkout_runtime_directory or base / "checkout-runtime"
        ).expanduser().resolve()
        self.checkout_profiles_directory = Path(
            checkout_profiles_directory or base / "checkout-profiles"
        ).expanduser().resolve()
        self.checkout_browser_executable = (
            Path(checkout_browser_executable).expanduser().resolve()
            if checkout_browser_executable
            else None
        )
        self._prepare_private_directory(self.checkout_runtime_directory)
        self._prepare_private_directory(self.checkout_profiles_directory)
        self._payment_session: subprocess.Popen[bytes] | None = None
        self._payment_session_expires_at = 0
        self._payment_session_max_operations = 0
        self._payment_session_lock = threading.RLock()
        if self.checkout_browser_executable is not None:
            self._require_owner_executable(self.checkout_browser_executable, "checkout browser")

    def _prepare_private_directory(self, path: Path) -> None:
        path.mkdir(mode=0o700, parents=True, exist_ok=True)
        metadata = path.lstat()
        if stat.S_ISLNK(metadata.st_mode) or not stat.S_ISDIR(metadata.st_mode):
            raise ValueError(f"{path} must be a directory, not a symlink")
        if metadata.st_mode & 0o077:
            raise ValueError(f"{path} permissions are too broad")

    def _require_cixa_binary(self) -> None:
        self._require_owner_executable(self.cixa_binary, "cixa binary")

    @staticmethod
    def _require_owner_executable(path: Path, label: str) -> None:
        if not path.is_absolute():
            raise ValueError(f"{label} must be an absolute path")
        metadata = path.lstat()
        if stat.S_ISLNK(metadata.st_mode) or not stat.S_ISREG(metadata.st_mode):
            raise ValueError(f"{label} must be a regular non-symlink file")
        if metadata.st_uid not in {0, os.geteuid()} or metadata.st_mode & 0o022 or not os.access(path, os.X_OK):
            raise ValueError(f"{label} must be root/owner-controlled, non-writable, and executable")
        parent = path.parent
        while parent != parent.parent:
            parent_metadata = parent.lstat()
            if (
                stat.S_ISLNK(parent_metadata.st_mode)
                or not stat.S_ISDIR(parent_metadata.st_mode)
                or parent_metadata.st_uid not in {0, os.geteuid()}
                or parent_metadata.st_mode & 0o022
            ):
                raise ValueError(f"{label} has an unsafe parent directory")
            parent = parent.parent

    def ensure_checkout_runtime(self) -> None:
        self._prepare_private_directory(self.checkout_runtime_directory)
        key = self.checkout_runtime_directory / "helper.key"
        helper_id = self.checkout_runtime_directory / "helper.id"
        if key.exists() or helper_id.exists():
            if not key.exists() or not helper_id.exists():
                raise ValueError("checkout helper initialization is incomplete")
            read_private_token(str(key))
            read_private_token(str(helper_id))
            return
        self._require_cixa_binary()
        subprocess.run(
            [
                str(self.cixa_binary),
                "init-helper",
                "--helper-dir",
                str(self.checkout_runtime_directory),
            ],
            check=True,
            stdin=subprocess.DEVNULL,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
            timeout=10,
        )

    def payment_session_status(self) -> dict[str, Any]:
        with self._payment_session_lock:
            process = self._payment_session
            active = process is not None and process.poll() is None
            if not active:
                if process is not None and process.stderr is not None:
                    process.stderr.close()
                self._payment_session = None
                self._payment_session_expires_at = 0
                self._payment_session_max_operations = 0
            return {
                "active": active,
                "expires_at": self._payment_session_expires_at if active else None,
                "max_operations": self._payment_session_max_operations if active else 0,
                "secret_persisted": False,
            }

    def stop_payment_session(self) -> dict[str, Any]:
        with self._payment_session_lock:
            process = self._payment_session
            if process is not None and process.poll() is None:
                process.terminate()
                try:
                    process.wait(timeout=2)
                except subprocess.TimeoutExpired:
                    process.kill()
                    process.wait(timeout=2)
            if process is not None and process.stderr is not None:
                process.stderr.close()
            self._payment_session = None
            self._payment_session_expires_at = 0
            self._payment_session_max_operations = 0
            (self.checkout_runtime_directory / "session.sock").unlink(missing_ok=True)
        return self.payment_session_status()

    @staticmethod
    def _luhn_valid(pan: str) -> bool:
        digits = [int(value) for value in pan]
        checksum = 0
        parity = len(digits) % 2
        for index, digit in enumerate(digits):
            if index % 2 == parity:
                digit *= 2
                if digit > 9:
                    digit -= 9
            checksum += digit
        return checksum % 10 == 0

    def arm_payment_session(self, body: dict[str, Any]) -> dict[str, Any]:
        if set(body) != {"pan", "expiry", "cvv", "cardholder", "ttl_secs", "max_operations"}:
            raise ValueError("payment session fields are invalid")
        pan = body["pan"]
        expiry = body["expiry"]
        cvv = body["cvv"]
        cardholder = body["cardholder"]
        ttl_secs = body["ttl_secs"]
        max_operations = body["max_operations"]
        if not isinstance(pan, str) or not pan.isascii() or not pan.isdigit() or not 12 <= len(pan) <= 19 or not self._luhn_valid(pan):
            raise ValueError("card number is invalid")
        if not isinstance(expiry, str) or re.fullmatch(r"(?:0[1-9]|1[0-2])/[0-9]{2}", expiry) is None:
            raise ValueError("expiry must use MM/YY")
        expiry_month, expiry_year = (int(value) for value in expiry.split("/"))
        current = time.localtime()
        expiry_year += 2000
        if (expiry_year, expiry_month) < (current.tm_year, current.tm_mon) or expiry_year > current.tm_year + 20:
            raise ValueError("card expiry is outside the supported range")
        if not isinstance(cvv, str) or not cvv.isascii() or not cvv.isdigit() or not 3 <= len(cvv) <= 4:
            raise ValueError("security code is invalid")
        if not isinstance(cardholder, str) or not 1 <= len(cardholder) <= 128 or any(ord(value) < 32 or ord(value) == 127 for value in cardholder):
            raise ValueError("cardholder is invalid")
        if not isinstance(ttl_secs, int) or isinstance(ttl_secs, bool) or not 60 <= ttl_secs <= 3600:
            raise ValueError("session duration must be within 60..3600 seconds")
        if not isinstance(max_operations, int) or isinstance(max_operations, bool) or not 1 <= max_operations <= 100:
            raise ValueError("session operation limit must be within 1..100")
        self.ensure_checkout_runtime()
        self._require_cixa_binary()
        self.stop_payment_session()
        socket_path = self.checkout_runtime_directory / "session.sock"
        if len(os.fsencode(socket_path)) >= 100:
            raise ValueError("checkout runtime path is too long for a Unix socket; choose a shorter absolute directory")
        secret = json.dumps(
            {"pan": pan, "expiry": expiry, "cvv": cvv, "cardholder": cardholder},
            separators=(",", ":"),
        ).encode("utf-8") + b"\n"
        with self._payment_session_lock:
            process = subprocess.Popen(
                [
                    str(self.cixa_binary),
                    "secret-session",
                    "--socket",
                    str(socket_path),
                    "--helper-key-file",
                    str(self.checkout_runtime_directory / "helper.key"),
                    "--helper-id-file",
                    str(self.checkout_runtime_directory / "helper.id"),
                    "--redemption-dir",
                    str(self.checkout_runtime_directory / "redeemed"),
                    "--ttl-secs",
                    str(ttl_secs),
                    "--max-operations",
                    str(max_operations),
                ],
                stdin=subprocess.PIPE,
                stdout=subprocess.DEVNULL,
                stderr=subprocess.PIPE,
            )
            assert process.stdin is not None
            process.stdin.write(secret)
            process.stdin.close()
            secret = b""
            deadline = time.monotonic() + 3
            while time.monotonic() < deadline and process.poll() is None and not socket_path.exists():
                time.sleep(0.02)
            if process.poll() is not None or not socket_path.exists():
                if process.poll() is None:
                    process.kill()
                    process.wait(timeout=2)
                detail = process.stderr.read(4096).decode("utf-8", errors="replace").strip() if process.stderr else ""
                if process.stderr is not None:
                    process.stderr.close()
                raise RuntimeError(
                    "payment session could not be armed"
                    + (f": {detail}" if detail else "")
                )
            self._payment_session = process
            self._payment_session_expires_at = int(time.time()) + ttl_secs
            self._payment_session_max_operations = max_operations
        return self.payment_session_status()

    @staticmethod
    def _canonical_origin(value: Any) -> str:
        if not isinstance(value, str) or not 1 <= len(value) <= 2048:
            raise ValueError("checkout origin is invalid")
        parsed = urlsplit(value)
        if parsed.scheme != "https" or not parsed.hostname or parsed.username or parsed.password or parsed.port not in {None, 443}:
            raise ValueError("checkout origins must be credential-free HTTPS URLs")
        if parsed.path not in {"", "/"} or parsed.query or parsed.fragment:
            raise ValueError("checkout origin must not contain a path, query, or fragment")
        return f"https://{parsed.hostname.lower()}"

    def list_checkout_profiles(self) -> list[dict[str, Any]]:
        self._prepare_private_directory(self.checkout_profiles_directory)
        profiles: list[dict[str, Any]] = []
        for path in sorted(self.checkout_profiles_directory.glob("*.json")):
            metadata = path.lstat()
            if stat.S_ISLNK(metadata.st_mode) or not stat.S_ISREG(metadata.st_mode) or metadata.st_mode & 0o077:
                raise ValueError("checkout profile permissions are invalid")
            value = json.loads(path.read_text(encoding="utf-8"))
            profiles.append(
                {
                    "profile_id": path.stem,
                    "merchant_domain": value["merchantDomain"],
                    "processor_origins": value["config"]["allowedProcessorOrigins"],
                    "timeout_ms": value["config"]["timeoutMs"],
                }
            )
        return profiles

    def save_checkout_profile(self, body: dict[str, Any]) -> dict[str, Any]:
        expected = {
            "profile_id",
            "merchant_domain",
            "browser_executable",
            "allowed_navigation_origins",
            "allowed_processor_origins",
            "selectors",
            "timeout_ms",
        }
        if set(body) != expected:
            raise ValueError("checkout profile fields are invalid")
        profile_id = body["profile_id"]
        merchant_domain = body["merchant_domain"]
        browser_executable = body["browser_executable"]
        timeout_ms = body["timeout_ms"]
        if not isinstance(profile_id, str) or PROFILE_ID.fullmatch(profile_id) is None:
            raise ValueError("profile identifier is invalid")
        if not isinstance(merchant_domain, str) or DOMAIN.fullmatch(merchant_domain.lower()) is None:
            raise ValueError("merchant domain is invalid")
        merchant_domain = merchant_domain.lower()
        if not isinstance(browser_executable, str) or not Path(browser_executable).is_absolute():
            raise ValueError("browser executable must be an absolute path")
        self._require_owner_executable(Path(browser_executable), "browser executable")
        if not isinstance(timeout_ms, int) or isinstance(timeout_ms, bool) or not 1_000 <= timeout_ms <= 120_000:
            raise ValueError("checkout timeout is invalid")
        navigation = body["allowed_navigation_origins"]
        processors = body["allowed_processor_origins"]
        if not isinstance(navigation, list) or not 1 <= len(navigation) <= 16:
            raise ValueError("navigation origins are invalid")
        if not isinstance(processors, list) or not 1 <= len(processors) <= 16:
            raise ValueError("processor origins are invalid")
        navigation = [self._canonical_origin(value) for value in navigation]
        processors = [self._canonical_origin(value) for value in processors]
        if f"https://{merchant_domain}" not in navigation:
            raise ValueError("navigation origins must include the merchant origin")
        if set(navigation) & set(processors):
            raise ValueError("processor and navigation origins must be disjoint")
        required_selectors = {
            "finalTotal",
            "currency",
            "fulfillment",
            "items",
            "recurring",
            "trialAutoRenew",
            "storedCard",
            "tipMinor",
            "preauthorization",
            "installments",
            "paymentFrame",
            "pan",
            "expiry",
            "cvv",
            "submit",
        }
        selectors = body["selectors"]
        selector_keys = set(selectors) if isinstance(selectors, dict) else set()
        if not isinstance(selectors, dict) or (
            selector_keys != required_selectors
            and selector_keys != required_selectors | {"cardholder"}
        ):
            raise ValueError("checkout selectors are incomplete")
        if any(not isinstance(value, str) or not 1 <= len(value) <= 512 or any(ord(character) < 32 for character in value) for value in selectors.values()):
            raise ValueError("checkout selector is invalid")
        profile = {
            "profileVersion": 1,
            "merchantDomain": merchant_domain,
            "config": {
                "browserExecutable": browser_executable,
                "checkoutUrl": f"https://{merchant_domain}/",
                "allowedNavigationOrigins": navigation,
                "allowedProcessorOrigins": processors,
                "selectors": selectors,
                "timeoutMs": timeout_ms,
            },
        }
        self._prepare_private_directory(self.checkout_profiles_directory)
        destination = self.checkout_profiles_directory / f"{profile_id}.json"
        temporary = self.checkout_profiles_directory / f".{profile_id}.{secrets.token_hex(8)}.tmp"
        descriptor = os.open(temporary, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
        try:
            payload = (json.dumps(profile, separators=(",", ":")) + "\n").encode("utf-8")
            written = 0
            while written < len(payload):
                count = os.write(descriptor, payload[written:])
                if count <= 0:
                    raise OSError("checkout profile write made no progress")
                written += count
            os.fsync(descriptor)
        finally:
            os.close(descriptor)
        os.replace(temporary, destination)
        directory = os.open(self.checkout_profiles_directory, os.O_RDONLY)
        try:
            os.fsync(directory)
        finally:
            os.close(directory)
        return {"saved": True, "profile_id": profile_id, "merchant_domain": merchant_domain}

    def delete_checkout_profile(self, body: dict[str, Any]) -> dict[str, Any]:
        if set(body) != {"profile_id"}:
            raise ValueError("checkout profile delete fields are invalid")
        profile_id = body["profile_id"]
        if not isinstance(profile_id, str) or PROFILE_ID.fullmatch(profile_id) is None:
            raise ValueError("profile identifier is invalid")
        destination = self.checkout_profiles_directory / f"{profile_id}.json"
        metadata = destination.lstat()
        if stat.S_ISLNK(metadata.st_mode) or not stat.S_ISREG(metadata.st_mode):
            raise ValueError("checkout profile is invalid")
        destination.unlink()
        directory = os.open(self.checkout_profiles_directory, os.O_RDONLY)
        try:
            os.fsync(directory)
        finally:
            os.close(directory)
        return {"deleted": True, "profile_id": profile_id}

    def checkout_status(self) -> dict[str, Any]:
        return {
            "runtime_initialized": (self.checkout_runtime_directory / "helper.key").exists()
            and (self.checkout_runtime_directory / "helper.id").exists(),
            "payment_session": self.payment_session_status(),
            "profiles": self.list_checkout_profiles(),
            "runtime_directory": str(self.checkout_runtime_directory),
            "profiles_directory": str(self.checkout_profiles_directory),
            "suggested_browser_executable": str(self.checkout_browser_executable)
            if self.checkout_browser_executable is not None
            else None,
        }

    def _prepare_agent_token_directory(self) -> None:
        self.agent_token_directory.mkdir(mode=0o700, parents=True, exist_ok=True)
        metadata = self.agent_token_directory.lstat()
        if stat.S_ISLNK(metadata.st_mode) or not stat.S_ISDIR(metadata.st_mode):
            raise ValueError("agent token directory must be a directory, not a symlink")
        expected_mode = 0o750 if self.agent_gid is not None else 0o700
        if self.agent_gid is not None:
            os.chown(self.agent_token_directory, -1, self.agent_gid)
        os.chmod(self.agent_token_directory, expected_mode)

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
            if self.agent_gid is not None:
                os.chown(token_path, -1, self.agent_gid)
                os.chmod(token_path, 0o640)
                with token_path.open("rb") as token_file:
                    os.fsync(token_file.fileno())
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
                    {"credential_reference", "provider_kind", "last_four", "balance", "balance_status", "balance_ttl_secs", "autonomous_checkout"},
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
            if path == "/api/checkout":
                if not self._require_owner():
                    return
                try:
                    self._send_json(200, state.checkout_status())
                except (OSError, RuntimeError, ValueError, json.JSONDecodeError):
                    self._send_json(502, {"error": "checkout runtime status is unavailable"})
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
                if self.path == "/api/checkout/setup":
                    if body:
                        raise ValueError("checkout setup body must be empty")
                    state.ensure_checkout_runtime()
                    self._send_json(200, state.checkout_status())
                    return
                if self.path == "/api/checkout/session/arm":
                    self._send_json(200, state.arm_payment_session(body))
                    return
                if self.path == "/api/checkout/session/stop":
                    if body:
                        raise ValueError("checkout stop body must be empty")
                    self._send_json(200, state.stop_payment_session())
                    return
                if self.path == "/api/checkout/profiles":
                    self._send_json(200, state.save_checkout_profile(body))
                    return
                if self.path == "/api/checkout/profiles/delete":
                    self._send_json(200, state.delete_checkout_profile(body))
                    return
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
            except ValueError as error:
                self._send_json(400, {"error": str(error)})
            except (OSError, RuntimeError, json.JSONDecodeError):
                self._send_json(400, {"error": "request rejected"})

    return Handler


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--socket-path", required=True)
    parser.add_argument("--owner-token-file", required=True)
    parser.add_argument("--access-token-file", required=True)
    parser.add_argument("--agent-token-directory")
    parser.add_argument("--agent-gid", type=int)
    parser.add_argument("--cixa-binary")
    parser.add_argument("--checkout-runtime-directory")
    parser.add_argument("--checkout-profiles-directory")
    parser.add_argument("--checkout-browser-executable")
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
        args.cixa_binary,
        args.checkout_runtime_directory,
        args.checkout_profiles_directory,
        args.checkout_browser_executable,
        args.agent_gid,
    )
    server = http.server.ThreadingHTTPServer(("127.0.0.1", args.port), make_handler(state))
    print(f"owner dashboard listening on http://127.0.0.1:{args.port}")
    try:
        server.serve_forever()
    except KeyboardInterrupt:
        pass
    finally:
        state.stop_payment_session()
        server.server_close()


if __name__ == "__main__":
    main()
