#!/usr/bin/env python3
"""Exercise the owner helper and persisted automated-handoff orchestration."""

from __future__ import annotations

import json
import os
import subprocess
import tempfile
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
BINARY = ROOT / "target" / "debug" / "treasury"
NODE = Path(subprocess.run(["which", "node"], check=True, capture_output=True, text=True).stdout.strip())


def run(*args: str) -> dict:
    result = subprocess.run([str(BINARY), *args], cwd=ROOT, check=True, capture_output=True, text=True)
    return json.loads(result.stdout)


with tempfile.TemporaryDirectory(prefix="agent-treasury-handoff-") as raw_directory:
    directory = Path(raw_directory)
    owner_file = directory / "owner.token"
    agent_file = directory / "agent.token"
    request_file = directory / "request.json"
    helper_dir = directory / "helper"
    helper_socket = helper_dir / "helper.sock"
    redemption_dir = helper_dir / "redeemed"
    adapter = directory / "adapter.mjs"
    adapter_config = directory / "adapter.json"

    run("init", "--data-dir", str(directory), "--owner-token-file", str(owner_file), "--balance-minor", "10000")
    run(
        "configure-manual-provider", "--data-dir", str(directory),
        "--owner-token-file", str(owner_file), "--credential-reference",
        "keychain://agent-treasury/handoff-test", "--provider-kind", "os-credential-store",
        "--last-four", "1111", "--balance-minor", "10000", "--balance-status", "owner_confirmed",
    )
    run(
        "create-agent", "--data-dir", str(directory), "--owner-token-file", str(owner_file),
        "--agent-token-file", str(agent_file), "--mode", "bounded_autonomous",
    )
    request_file.write_text(json.dumps({
        "idempotency_key": "automated-handoff", "amount": {"minor": 500, "currency": "CAD"},
        "final_total": {"minor": 500, "currency": "CAD"}, "merchant_domain": "merchant.example.test",
        "category": "software", "items": [{"label": "handoff item", "quantity": 1, "unit_price_minor": 500}],
        "recurring": False, "trial_auto_renew": False, "stored_card": False,
        "tip_minor": 0, "preauthorization": False, "installments": False,
        "fulfillment_profile": "digital-email", "payment_form": "hosted_fields",
        "redirect_chain": ["https://merchant.example.test/checkout"], "attempts": 1,
        "session_id": "automated-handoff", "scenario": "normal",
    }), encoding="utf-8")
    intent = run(
        "intent", "--data-dir", str(directory), "--token-file", str(agent_file),
        "--request-file", str(request_file),
    )
    run(
        "approve", "--data-dir", str(directory), "--owner-token-file", str(owner_file),
        "--intent-id", intent["id"],
    )
    run("init-helper", "--helper-dir", str(helper_dir))
    adapter.write_text("""
import readline from 'node:readline';
const lines = readline.createInterface({input: process.stdin, terminal: false});
for await (const line of lines) {
  const value = JSON.parse(line);
  if (value.secret.pan !== 'synthetic-pan' || value.request.final_total.minor !== 500) process.exit(3);
  process.stdout.write(JSON.stringify({outcome: 'approved', reference: 'handoff-ref-1'}) + '\\n');
  break;
}
""".strip() + "\n", encoding="utf-8")
    adapter_config.write_text("{}\n", encoding="utf-8")
    os.chmod(adapter, 0o600)
    os.chmod(adapter_config, 0o600)
    helper = subprocess.Popen([
        str(BINARY), "secret-helper", "--socket", str(helper_socket),
        "--helper-key-file", str(helper_dir / "helper.key"),
        "--helper-id-file", str(helper_dir / "helper.id"),
        "--redemption-dir", str(redemption_dir),
    ], cwd=ROOT, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    assert helper.stdin is not None
    helper.stdin.write(b'{"pan":"synthetic-pan","expiry":"12/99","cvv":"999"}\n')
    helper.stdin.close()
    for _ in range(100):
        if helper_socket.exists():
            break
        time.sleep(0.02)
    assert helper_socket.exists(), "owner helper did not bind"
    executed = run(
        "execute-handoff", "--data-dir", str(directory), "--owner-token-file", str(owner_file),
        "--intent-id", intent["id"], "--helper-socket", str(helper_socket),
        "--helper-key-file", str(helper_dir / "helper.key"),
        "--helper-id-file", str(helper_dir / "helper.id"), "--node-path", str(NODE),
        "--adapter-script", str(adapter), "--adapter-config", str(adapter_config),
    )
    assert executed["status"] == "settled"
    assert helper.wait(timeout=5) == 0
    assert any(redemption_dir.iterdir()), "helper did not durably redeem the grant"
    assert "synthetic-pan" not in (directory / "state.json").read_text(encoding="utf-8")
    print("owner helper and automated handoff assertions passed")
