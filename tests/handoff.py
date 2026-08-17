#!/usr/bin/env python3
"""Exercise the owner helper and persisted automated-handoff orchestration."""

from __future__ import annotations

import atexit
import json
import os
import shutil
import subprocess
import tempfile
import time
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
BINARY = ROOT / "target" / "debug" / "treasury"
NODE = Path(subprocess.run(["which", "node"], check=True, capture_output=True, text=True).stdout.strip())
ARTIFACTS = ROOT / "build" / "handoff-artifacts"
shutil.rmtree(ARTIFACTS, ignore_errors=True)
ARTIFACTS.mkdir(parents=True)
command_index = 0


def scan_handoff_artifacts() -> None:
    if not any(ARTIFACTS.iterdir()):
        return
    result = subprocess.run(
        [sys.executable, str(ROOT / "scripts" / "secret-canary-scan.py"), str(ARTIFACTS)],
        cwd=ROOT,
        capture_output=True,
        text=True,
    )
    if result.returncode != 0:
        sys.stderr.write(result.stdout + result.stderr)
        sys.stderr.flush()
        os._exit(1)


atexit.register(scan_handoff_artifacts)


def run(*args: str) -> dict:
    global command_index
    result = subprocess.run([str(BINARY), *args], cwd=ROOT, capture_output=True, text=True)
    command_index += 1
    (ARTIFACTS / f"command-{command_index}.stdout").write_text(result.stdout, encoding="utf-8")
    (ARTIFACTS / f"command-{command_index}.stderr").write_text(result.stderr, encoding="utf-8")
    result.check_returncode()
    return json.loads(result.stdout)


TEST_TEMP_ROOT = ROOT / "build"
TEST_TEMP_ROOT.mkdir(exist_ok=True)

with tempfile.TemporaryDirectory(prefix="handoff-", dir=TEST_TEMP_ROOT) as raw_directory:
    directory = Path(raw_directory).resolve()
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
  if (value.secret.pan !== '4111111111111111' || value.request.final_total.minor !== 500) process.exit(3);
  process.stdout.write(JSON.stringify({outcome: 'approved', reference: 'handoff-ref-1'}) + '\\n');
  break;
}
""".strip() + "\n", encoding="utf-8")
    adapter_config.write_text(
        json.dumps({"browserExecutable": str(NODE), "timeoutMs": 1000}) + "\n",
        encoding="utf-8",
    )
    os.chmod(adapter, 0o600)
    os.chmod(adapter_config, 0o600)
    helper = subprocess.Popen([
        str(BINARY), "secret-helper", "--socket", str(helper_socket),
        "--helper-key-file", str(helper_dir / "helper.key"),
        "--helper-id-file", str(helper_dir / "helper.id"),
        "--redemption-dir", str(redemption_dir),
    ], cwd=ROOT, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    assert helper.stdin is not None
    helper.stdin.write(b'{"pan":"4111111111111111","expiry":"12/99","cvv":"737"}\n')
    helper.stdin.close()
    for _ in range(100):
        if helper_socket.exists():
            break
        time.sleep(0.02)
    if not helper_socket.exists():
        assert helper.stderr is not None
        raise AssertionError(f"owner helper did not bind: {helper.stderr.read().decode()}")
    executed = run(
        "execute-handoff", "--data-dir", str(directory), "--owner-token-file", str(owner_file),
        "--intent-id", intent["id"], "--helper-socket", str(helper_socket),
        "--helper-key-file", str(helper_dir / "helper.key"),
        "--helper-id-file", str(helper_dir / "helper.id"), "--node-path", str(NODE),
        "--adapter-script", str(adapter), "--adapter-config", str(adapter_config),
    )
    assert executed["status"] == "unknown"
    assert helper.wait(timeout=5) == 0
    assert helper.stdout is not None and helper.stderr is not None
    (ARTIFACTS / "helper-success.stdout").write_bytes(helper.stdout.read())
    (ARTIFACTS / "helper-success.stderr").write_bytes(helper.stderr.read())
    assert any(redemption_dir.iterdir()), "helper did not durably redeem the grant"
    assert "4111111111111111" not in (directory / "state.json").read_text(encoding="utf-8")

    timeout_request = json.loads(request_file.read_text(encoding="utf-8"))
    timeout_request["idempotency_key"] = "automated-handoff-timeout"
    timeout_request["amount"]["minor"] = 501
    timeout_request["final_total"]["minor"] = 501
    timeout_request["items"][0]["unit_price_minor"] = 501
    request_file.write_text(json.dumps(timeout_request), encoding="utf-8")
    timeout_intent = run(
        "intent", "--data-dir", str(directory), "--token-file", str(agent_file),
        "--request-file", str(request_file),
    )
    run(
        "approve", "--data-dir", str(directory), "--owner-token-file", str(owner_file),
        "--intent-id", timeout_intent["id"],
    )
    adapter.write_text("""
import { spawn } from 'node:child_process';
spawn(process.execPath, ['-e', 'setInterval(() => {}, 1000)'], {
  stdio: ['ignore', process.stdout, process.stderr],
});
process.exit(0);
""".strip() + "\n", encoding="utf-8")
    helper = subprocess.Popen([
        str(BINARY), "secret-helper", "--socket", str(helper_socket),
        "--helper-key-file", str(helper_dir / "helper.key"),
        "--helper-id-file", str(helper_dir / "helper.id"),
        "--redemption-dir", str(redemption_dir),
    ], cwd=ROOT, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    assert helper.stdin is not None
    helper.stdin.write(b'{"pan":"4111111111111111","expiry":"12/99","cvv":"737"}\n')
    helper.stdin.close()
    for _ in range(100):
        if helper_socket.exists():
            break
        time.sleep(0.02)
    started = time.monotonic()
    timed_out = subprocess.run([
        str(BINARY), "execute-handoff", "--data-dir", str(directory),
        "--owner-token-file", str(owner_file), "--intent-id", timeout_intent["id"],
        "--helper-socket", str(helper_socket), "--helper-key-file", str(helper_dir / "helper.key"),
        "--helper-id-file", str(helper_dir / "helper.id"), "--node-path", str(NODE),
        "--adapter-script", str(adapter), "--adapter-config", str(adapter_config),
    ], cwd=ROOT, capture_output=True, text=True, timeout=8)
    assert timed_out.returncode != 0
    (ARTIFACTS / "execute-timeout.stdout").write_text(timed_out.stdout, encoding="utf-8")
    (ARTIFACTS / "execute-timeout.stderr").write_text(timed_out.stderr, encoding="utf-8")
    assert time.monotonic() - started < 7
    assert helper.wait(timeout=5) == 0
    assert helper.stdout is not None and helper.stderr is not None
    (ARTIFACTS / "helper-timeout.stdout").write_bytes(helper.stdout.read())
    (ARTIFACTS / "helper-timeout.stderr").write_bytes(helper.stderr.read())
    persisted = json.loads((directory / "state.json").read_text(encoding="utf-8"))
    assert persisted["state"]["intents"][timeout_intent["id"]]["state"] == "unknown"
    assert "4111111111111111" not in json.dumps(persisted)

    def create_approved_intent(key: str, minor: int) -> dict:
        candidate = json.loads(request_file.read_text(encoding="utf-8"))
        candidate["idempotency_key"] = key
        candidate["amount"]["minor"] = minor
        candidate["final_total"]["minor"] = minor
        candidate["items"][0]["unit_price_minor"] = minor
        request_file.write_text(json.dumps(candidate), encoding="utf-8")
        created = run(
            "intent", "--data-dir", str(directory), "--token-file", str(agent_file),
            "--request-file", str(request_file),
        )
        run(
            "approve", "--data-dir", str(directory), "--owner-token-file", str(owner_file),
            "--intent-id", created["id"],
        )
        return created

    def launch_helper(secret: bytes) -> subprocess.Popen[bytes]:
        process = subprocess.Popen([
            str(BINARY), "secret-helper", "--socket", str(helper_socket),
            "--helper-key-file", str(helper_dir / "helper.key"),
            "--helper-id-file", str(helper_dir / "helper.id"),
            "--redemption-dir", str(redemption_dir),
        ], cwd=ROOT, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
        assert process.stdin is not None
        process.stdin.write(secret + b"\n")
        process.stdin.close()
        for _ in range(100):
            if helper_socket.exists():
                break
            time.sleep(0.02)
        assert helper_socket.exists(), "owner helper did not bind"
        return process

    injection_intent = create_approved_intent("automated-handoff-json-injection", 502)
    adapter_marker = directory / "adapter-ran"
    adapter.write_text(
        f"import {{writeFileSync}} from 'node:fs'; writeFileSync({json.dumps(str(adapter_marker))}, 'ran');\n",
        encoding="utf-8",
    )
    helper = launch_helper(
        b'{},"request":{"final_total":{"minor":1,"currency":"CAD"}}'
    )
    injected = subprocess.run([
        str(BINARY), "execute-handoff", "--data-dir", str(directory),
        "--owner-token-file", str(owner_file), "--intent-id", injection_intent["id"],
        "--helper-socket", str(helper_socket), "--helper-key-file", str(helper_dir / "helper.key"),
        "--helper-id-file", str(helper_dir / "helper.id"), "--node-path", str(NODE),
        "--adapter-script", str(adapter), "--adapter-config", str(adapter_config),
    ], cwd=ROOT, capture_output=True, text=True, timeout=8)
    assert injected.returncode != 0
    assert helper.wait(timeout=5) == 0
    assert not adapter_marker.exists(), "malformed secret reached the adapter"

    detached_intent = create_approved_intent("automated-handoff-detached-output", 503)
    detached_pid_file = directory / "detached.pid"
    adapter.write_text(f"""
import {{spawn}} from 'node:child_process';
import {{writeFileSync}} from 'node:fs';
const child = spawn(process.execPath, ['-e', 'setTimeout(() => process.exit(0), 6000)'], {{
  detached: true, stdio: ['ignore', process.stdout, 'ignore'],
}});
child.unref();
writeFileSync({json.dumps(str(detached_pid_file))}, String(child.pid));
setTimeout(() => process.stdout.write(JSON.stringify({{outcome: 'unknown', reason: 'test'}}) + '\\n'), 100);
""".strip() + "\n", encoding="utf-8")
    helper = launch_helper(b'{"pan":"detached-canary","expiry":"12/99","cvv":"999"}')
    started = time.monotonic()
    detached = subprocess.run([
        str(BINARY), "execute-handoff", "--data-dir", str(directory),
        "--owner-token-file", str(owner_file), "--intent-id", detached_intent["id"],
        "--helper-socket", str(helper_socket), "--helper-key-file", str(helper_dir / "helper.key"),
        "--helper-id-file", str(helper_dir / "helper.id"), "--node-path", str(NODE),
        "--adapter-script", str(adapter), "--adapter-config", str(adapter_config),
    ], cwd=ROOT, capture_output=True, text=True, timeout=8)
    assert detached.returncode != 0
    (ARTIFACTS / "execute-detached.stdout").write_text(detached.stdout, encoding="utf-8")
    (ARTIFACTS / "execute-detached.stderr").write_text(detached.stderr, encoding="utf-8")
    assert time.monotonic() - started < 7
    assert helper.wait(timeout=5) == 0
    detached_pid = int(detached_pid_file.read_text(encoding="utf-8"))
    for _ in range(100):
        try:
            os.kill(detached_pid, 0)
        except ProcessLookupError:
            break
        time.sleep(0.01)
    else:
        raise AssertionError("detached checkout descendant survived broker cleanup")
    persisted = json.loads((directory / "state.json").read_text(encoding="utf-8"))
    assert persisted["state"]["intents"][detached_intent["id"]]["state"] == "unknown"
    assert "detached-canary" not in json.dumps(persisted)
    print("owner helper and automated handoff assertions passed")
