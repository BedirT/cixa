#!/usr/bin/env python3
"""Exercise the persisted daemon, Unix socket, Python SDK, and owner boundary."""

from __future__ import annotations

import json
import os
import signal
import subprocess
import tempfile
import time
from pathlib import Path

from agent_treasury import BrokerError, TreasuryClient


ROOT = Path(__file__).resolve().parents[1]
BINARY = ROOT / "target" / "debug" / "treasury"


def run(*args: str) -> dict:
    return json.loads(subprocess.run([str(BINARY), *args], cwd=ROOT, check=True, capture_output=True, text=True).stdout)


with tempfile.TemporaryDirectory(prefix="agent-treasury-integration-") as raw_directory:
    directory = Path(raw_directory)
    owner_file = directory / "owner.token"
    agent_file = directory / "agent.token"
    run("init", "--data-dir", str(directory), "--owner-token-file", str(owner_file), "--balance-minor", "10000")
    run("create-agent", "--data-dir", str(directory), "--owner-token-file", str(owner_file), "--agent-token-file", str(agent_file), "--mode", "bounded_autonomous")
    run("configure-receive", "--data-dir", str(directory), "--owner-token-file", str(owner_file), "--address", "public-inbox@example.invalid")
    for protected in (
        owner_file,
        agent_file,
        directory / "state.json",
        directory / "audit.key",
        directory / "treasury.lock",
    ):
        assert protected.stat().st_mode & 0o077 == 0, protected
    socket_path = directory / "treasury.sock"
    daemon = subprocess.Popen([str(BINARY), "serve", "--data-dir", str(directory), "--socket", str(socket_path)], cwd=ROOT, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
    try:
        for _ in range(100):
            if socket_path.exists():
                break
            time.sleep(0.05)
        if not socket_path.exists():
            raise SystemExit("daemon did not create its Unix socket")
        client = TreasuryClient(str(socket_path), str(agent_file))
        assert client.get_status()["principal"] == "agent"
        assert client.get_receive_instructions()["outgoing_transfers_supported"] is False
        request = {
            "idempotency_key": "integration-purchase",
            "amount": {"minor": 500, "currency": "CAD"},
            "final_total": {"minor": 500, "currency": "CAD"},
            "merchant_domain": "merchant.example.test",
            "category": "software",
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
            "session_id": "integration-session",
            "scenario": "normal",
        }
        intent = client.create_purchase_intent(request)
        executed = client.execute_purchase_intent(intent["id"])
        assert executed["status"] == "settled"
        assert client.create_purchase_intent(request)["id"] == intent["id"]
        run(
            "stop",
            "--data-dir",
            str(directory),
            "--owner-token-file",
            str(owner_file),
        )
        stopped_request = dict(request)
        stopped_request["idempotency_key"] = "integration-stopped"
        assert client.create_purchase_intent(stopped_request)["state"] == "failed"
        run(
            "resume",
            "--data-dir",
            str(directory),
            "--owner-token-file",
            str(owner_file),
        )
        try:
            client.request({"type": "owner_set_emergency_stop", "stopped": True})
        except BrokerError:
            pass
        else:
            raise SystemExit("agent capability reached an owner operation")
        assert client.get_receipt(intent["id"])["personal_information_redacted"] is True
        daemon.send_signal(signal.SIGTERM)
        daemon.wait(timeout=5)
        socket_path.unlink(missing_ok=True)
        daemon = subprocess.Popen([str(BINARY), "serve", "--data-dir", str(directory), "--socket", str(socket_path)], cwd=ROOT, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
        for _ in range(100):
            if socket_path.exists():
                break
            time.sleep(0.05)
        assert TreasuryClient(str(socket_path), str(agent_file)).get_receipt(intent["id"])["intent_id"] == intent["id"]
        run(
            "revoke-agent",
            "--data-dir",
            str(directory),
            "--owner-token-file",
            str(owner_file),
            "--agent-id",
            client.get_status()["agent_id"],
        )
        try:
            client.get_status()
        except BrokerError:
            pass
        else:
            raise SystemExit("revoked agent remained authorized")
        print("persisted daemon integration assertions passed")
    finally:
        daemon.send_signal(signal.SIGTERM)
        daemon.wait(timeout=5)
