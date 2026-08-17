"""Small, dependency-free client for the agent-scoped local v1 broker API."""

from __future__ import annotations

import json
import os
import socket
import uuid
from pathlib import Path
from typing import Any


class BrokerError(RuntimeError):
    """The broker rejected a request or the local IPC channel failed."""


def _bounded(value: str, field: str, maximum: int) -> None:
    if not value or len(value) > maximum or any(ord(char) < 32 or ord(char) == 127 for char in value):
        raise ValueError(f"{field} must contain 1..{maximum} printable characters")


class CixaClient:
    """Agent-only client. It reads a token from a protected file, never from arguments."""

    def __init__(self, socket_path: str, token_file: str, timeout: float = 10.0, execute_timeout: float = 180.0) -> None:
        _bounded(socket_path, "socket_path", 4096)
        _bounded(token_file, "token_file", 4096)
        self.socket_path = socket_path
        self.token = Path(token_file).read_text(encoding="utf-8").strip()
        _bounded(self.token, "capability token", 128)
        self.timeout = timeout
        self.execute_timeout = execute_timeout

    def request(self, operation: dict[str, Any], timeout: float | None = None) -> Any:
        envelope = {
            "api_version": "v1",
            "request_id": str(uuid.uuid4()),
            "token": self.token,
            "operation": operation,
        }
        try:
            with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as channel:
                channel.settimeout(self.timeout if timeout is None else timeout)
                channel.connect(self.socket_path)
                channel.sendall((json.dumps(envelope, separators=(",", ":")) + "\n").encode("utf-8"))
                response = b""
                while b"\n" not in response:
                    response += channel.recv(64 * 1024)
                    if len(response) > 256 * 1024:
                        raise BrokerError("broker response is too large")
                    if not response:
                        raise BrokerError("broker closed the IPC channel without a response")
        except OSError as error:
            raise BrokerError(f"broker connection failed: {error}") from error
        try:
            decoded = json.loads(response.split(b"\n", 1)[0].decode("utf-8"))
        except (UnicodeDecodeError, json.JSONDecodeError) as error:
            raise BrokerError("broker returned invalid JSON") from error
        if not decoded.get("ok"):
            raise BrokerError(decoded.get("error", "broker rejected request"))
        return decoded.get("data")

    def get_status(self) -> dict[str, Any]:
        return self.request({"type": "get_status"})

    def get_capabilities(self) -> dict[str, Any]:
        return self.request({"type": "get_capabilities"})

    def get_budget(self) -> dict[str, Any]:
        return self.request({"type": "get_budget"})

    def get_receive_instructions(self) -> dict[str, Any]:
        return self.request({"type": "get_receive_instructions"})

    def create_purchase_intent(self, request: dict[str, Any]) -> dict[str, Any]:
        for field in ("idempotency_key", "merchant_domain", "category", "fulfillment_profile", "session_id"):
            _bounded(str(request.get(field, "")), field, 253 if field == "merchant_domain" else 128)
        for field in ("amount", "final_total"):
            money = request.get(field)
            if not isinstance(money, dict) or not isinstance(money.get("minor"), int) or money["minor"] <= 0:
                raise ValueError(f"{field}.minor must be a positive integer")
            if not isinstance(money.get("currency"), str) or len(money["currency"]) != 3 or not money["currency"].isupper():
                raise ValueError(f"{field}.currency must be an uppercase ISO 4217 code")
        items = request.get("items")
        if not isinstance(items, list) or not 1 <= len(items) <= 50:
            raise ValueError("items must contain 1..50 entries")
        for item in items:
            if not isinstance(item, dict):
                raise ValueError("each item must be an object")
            _bounded(str(item.get("label", "")), "item.label", 160)
            if not isinstance(item.get("quantity"), int) or not 1 <= item["quantity"] <= 10_000:
                raise ValueError("item.quantity is invalid")
            if not isinstance(item.get("unit_price_minor"), int) or item["unit_price_minor"] < 0:
                raise ValueError("item.unit_price_minor is invalid")
        return self.request({"type": "create_purchase_intent", "request": request})

    def get_purchase_intent(self, intent_id: str) -> dict[str, Any]:
        _bounded(intent_id, "intent_id", 128)
        return self.request({"type": "get_purchase_intent", "intent_id": intent_id})

    def execute_purchase_intent(self, intent_id: str) -> dict[str, Any]:
        _bounded(intent_id, "intent_id", 128)
        return self.request({"type": "execute_purchase_intent", "intent_id": intent_id}, self.execute_timeout)

    def cancel_purchase_intent(self, intent_id: str) -> dict[str, Any]:
        _bounded(intent_id, "intent_id", 128)
        return self.request({"type": "cancel_purchase_intent", "intent_id": intent_id})

    def list_transactions(self, cursor: str | None = None, limit: int = 25) -> dict[str, Any]:
        if cursor is not None:
            _bounded(cursor, "cursor", 128)
        if not isinstance(limit, int) or isinstance(limit, bool) or not 1 <= limit <= 50:
            raise ValueError("limit must be an integer between 1 and 50")
        return self.request({"type": "list_transactions_page", "cursor": cursor, "limit": limit})

    def get_receipt(self, intent_id: str) -> dict[str, Any]:
        _bounded(intent_id, "intent_id", 128)
        return self.request({"type": "get_receipt", "intent_id": intent_id})
