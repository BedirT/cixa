#!/usr/bin/env python3
"""Exercise the persisted daemon, Unix socket, Python SDK, and owner boundary."""

from __future__ import annotations

import json
import os
import signal
import socket
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
    unsafe_init = subprocess.run(
        [str(BINARY), "init", "--data-dir", str(directory / "unsafe-init")],
        cwd=ROOT,
        capture_output=True,
        text=True,
    )
    assert unsafe_init.returncode != 0
    assert not (directory / "unsafe-init" / "state.json").exists()
    run("init", "--data-dir", str(directory), "--owner-token-file", str(owner_file), "--balance-minor", "10000")
    created = run("create-agent", "--data-dir", str(directory), "--owner-token-file", str(owner_file), "--agent-token-file", str(agent_file), "--mode", "bounded_autonomous")
    policy = {
        "id": "ignored-by-update",
        "version": 0,
        "primary_currency": "CAD",
        "max_per_transaction": {"minor": 2000, "currency": "CAD"},
        "max_per_session": {"minor": 5000, "currency": "CAD"},
        "max_rolling_24h": {"minor": 10000, "currency": "CAD"},
        "max_lifetime": {"minor": 25000, "currency": "CAD"},
        "absolute_exposure_ceiling": {"minor": 10000, "currency": "CAD"},
        "max_treasury_size": {"minor": 100000, "currency": "CAD"},
        "reinvestment_ratio_bps": 0,
        "allowed_currencies": ["CAD"],
        "allowed_merchants": ["merchant.example.test"],
        "denied_merchants": [],
        "approved_redirect_domains": [],
        "require_approval_for_new_merchants": True,
        "approved_fulfillment_profiles": ["digital-email"],
        "allow_recurring": False,
        "allow_trials": False,
        "allow_stored_card": False,
        "allow_tips": False,
        "allow_preauthorization": False,
        "allow_installments": False,
        "denied_categories": ["gambling", "crypto", "financial_transfer", "cash_withdrawal", "gift_card", "cash_equivalent"],
        "max_order_total_drift_minor": 0,
        "max_attempts": 1,
        "max_transactions_per_minute": 10,
        "max_redirects": 2,
        "intent_ttl_secs": 900,
        "card_session_ttl_secs": 600,
    }
    policy_file = directory / "policy.json"
    policy_file.write_text(json.dumps(policy), encoding="utf-8")
    updated = run("update-policy", "--data-dir", str(directory), "--owner-token-file", str(owner_file), "--agent-id", created["agent_id"], "--policy-file", str(policy_file))
    assert updated["policy"]["version"] == 2
    approved = run("approve-merchant", "--data-dir", str(directory), "--owner-token-file", str(owner_file), "--agent-id", created["agent_id"], "--merchant-domain", "Second.Example.Test")
    assert approved["merchant_domain"] == "second.example.test"
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
    owner_socket_path = directory / "owner.sock"
    daemon = subprocess.Popen([str(BINARY), "serve", "--data-dir", str(directory), "--socket", str(socket_path)], cwd=ROOT, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
    try:
        for _ in range(100):
            if socket_path.exists() and owner_socket_path.exists():
                break
            time.sleep(0.05)
        if not socket_path.exists() or not owner_socket_path.exists():
            raise SystemExit("daemon did not create its separate agent and owner sockets")
        flood = []
        for _ in range(32):
            connection = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
            connection.connect(str(socket_path))
            flood.append(connection)
        time.sleep(0.1)
        assert run("stop", "--data-dir", str(directory), "--owner-token-file", str(owner_file))["emergency_stop"] is True
        assert run("resume", "--data-dir", str(directory), "--owner-token-file", str(owner_file))["emergency_stop"] is False
        run(
            "arm-session",
            "--data-dir", str(directory),
            "--owner-token-file", str(owner_file),
            "--agent-id", created["agent_id"],
            "--ttl-secs", "300",
        )
        for connection in flood:
            connection.close()
        client = TreasuryClient(str(socket_path), str(agent_file))
        for _ in range(100):
            try:
                status = client.get_status()
                break
            except BrokerError:
                time.sleep(0.05)
        else:
            raise SystemExit("agent connection pool did not recover after flood connections closed")
        assert status["principal"] == "agent"
        try:
            client.request({"type": "get_status", "unexpected": True})
        except BrokerError:
            pass
        else:
            raise SystemExit("strict RPC schema accepted an unknown operation field")
        assert client.get_receive_instructions()["outgoing_transfers_supported"] is False
        request = {
            "idempotency_key": "integration-purchase",
            "amount": {"minor": 500, "currency": "CAD"},
            "final_total": {"minor": 500, "currency": "CAD"},
            "merchant_domain": "merchant.example.test",
            "category": "software",
            "items": [{"label": "integration item", "quantity": 1, "unit_price_minor": 500}],
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
