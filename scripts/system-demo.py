#!/usr/bin/env python3
"""Run the complete local simulated system through its public process boundaries."""

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
sys.path.insert(0, str(ROOT / "packages" / "sdk-python"))
from agent_treasury import BrokerError, TreasuryClient  # noqa: E402

BINARY = ROOT / "target" / "debug" / "treasury"


def run(*args: str) -> dict:
    result = subprocess.run([str(BINARY), *args], cwd=ROOT, check=True, capture_output=True, text=True)
    return json.loads(result.stdout)


def free_port() -> int:
    with socket.socket() as probe:
        probe.bind(("127.0.0.1", 0))
        return int(probe.getsockname()[1])


def wait_path(path: Path) -> None:
    for _ in range(100):
        if path.exists():
            return
        time.sleep(0.05)
    raise RuntimeError(f"service path was not created: {path}")


def http_status(port: int, authorization: str | None = None) -> int:
    connection = http.client.HTTPConnection("127.0.0.1", port, timeout=5)
    headers = {"Authorization": authorization} if authorization else {}
    connection.request("GET", "/", headers=headers)
    status = connection.getresponse().status
    connection.close()
    return status


def purchase(key: str, amount: int = 500) -> dict:
    return {
        "idempotency_key": key,
        "amount": {"minor": amount, "currency": "CAD"},
        "final_total": {"minor": amount, "currency": "CAD"},
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
        "session_id": "demo-caller-metadata",
        "scenario": "normal",
    }


def write_artifact(path: Path, value: object) -> None:
    path.write_text(json.dumps(value, indent=2) + "\n", encoding="utf-8")


with tempfile.TemporaryDirectory(prefix="agent-treasury-system-demo-") as raw_directory:
    directory = Path(raw_directory)
    owner_file = directory / "owner.token"
    agent_file = directory / "agent.token"
    access_file = directory / "dashboard.token"
    socket_path = directory / "treasury.sock"
    owner_socket = directory / "owner.sock"
    artifacts = directory / "artifacts"
    artifacts.mkdir(mode=0o700)
    access_token = "synthetic-dashboard-access-only"
    access_file.write_text(access_token + "\n", encoding="utf-8")
    os.chmod(access_file, 0o600)
    run("init", "--data-dir", str(directory), "--owner-token-file", str(owner_file), "--balance-minor", "10000")
    created = run(
        "create-agent", "--data-dir", str(directory), "--owner-token-file", str(owner_file),
        "--agent-token-file", str(agent_file), "--mode", "bounded_autonomous",
    )
    run(
        "configure-receive", "--data-dir", str(directory), "--owner-token-file", str(owner_file),
        "--address", "public-inbox@example.invalid",
    )
    merchant_port = free_port()
    dashboard_port = free_port()
    processes: list[subprocess.Popen] = []
    artifact_handles = []

    def start_process(name: str, command: list[str]) -> None:
        output = (artifacts / f"{name}.log").open("wb")
        artifact_handles.append(output)
        processes.append(subprocess.Popen(command, cwd=ROOT, stdout=output, stderr=output))

    def stop_processes() -> None:
        for process in reversed(processes):
            if process.poll() is None:
                process.send_signal(signal.SIGTERM)
        for process in reversed(processes):
            if process.poll() is None:
                try:
                    process.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    process.kill()
                    process.wait(timeout=5)
        for handle in artifact_handles:
            if not handle.closed:
                handle.close()

    try:
        start_process(
            "daemon",
            [str(BINARY), "serve", "--data-dir", str(directory), "--socket", str(socket_path)],
        )
        wait_path(socket_path)
        wait_path(owner_socket)
        start_process(
            "merchant",
            [
                sys.executable,
                "-m",
                "http.server",
                str(merchant_port),
                "--bind",
                "127.0.0.1",
                "--directory",
                str(ROOT / "apps" / "test-merchant"),
            ],
        )
        start_process(
            "dashboard",
            [
                sys.executable,
                str(ROOT / "apps" / "owner-dashboard" / "server.py"),
                "--socket-path",
                str(owner_socket),
                "--owner-token-file",
                str(owner_file),
                "--access-token-file",
                str(access_file),
                "--port",
                str(dashboard_port),
            ],
        )
        time.sleep(0.2)
        assert http_status(merchant_port) == 200
        auth = base64.b64encode(f"owner:{access_token}".encode()).decode()
        assert http_status(dashboard_port, f"Basic {auth}") == 200

        mcp_result = subprocess.run(
            ["node", str(ROOT / "scripts" / "demo-mcp.mjs"),
             str(ROOT / "packages" / "mcp-server" / "dist" / "index.js"),
             str(socket_path), str(agent_file)],
            cwd=ROOT, check=True, capture_output=True, text=True,
        )
        (artifacts / "mcp.stdout").write_text(mcp_result.stdout, encoding="utf-8")
        (artifacts / "mcp.stderr").write_text(mcp_result.stderr, encoding="utf-8")
        mcp = json.loads(mcp_result.stdout)
        client = TreasuryClient(str(socket_path), str(agent_file))
        starting_budget = client.get_budget()
        receiving = client.get_receive_instructions()
        valid = client.create_purchase_intent(purchase("demo-valid"))
        settled = client.execute_purchase_intent(valid["id"])
        receipt = client.get_receipt(valid["id"])
        remaining_budget = client.get_budget()
        duplicate = client.create_purchase_intent(purchase("demo-valid"))
        try:
            client.execute_purchase_intent(duplicate["id"])
            duplicate_execution = {"rejected": False}
        except BrokerError as error:
            expected_error = "conflict: intent is not executable in state Settled"
            duplicate_execution = {"rejected": str(error) == expected_error}
            if not duplicate_execution["rejected"]:
                raise RuntimeError("duplicate execution failed for an unexpected reason") from error
        after_duplicate = client.get_purchase_intent(valid["id"])
        if (
            after_duplicate["state"] != "settled"
            or after_duplicate["receipt_hash"] != settled["intent"]["receipt_hash"]
        ):
            raise RuntimeError("duplicate execution changed the settled intent")

        over_budget = client.create_purchase_intent(purchase("demo-over-budget", 3000))
        recurring_request = purchase("demo-recurring")
        recurring_request["recurring"] = True
        recurring = client.create_purchase_intent(recurring_request)
        currency_request = purchase("demo-currency")
        currency_request["amount"] = {"minor": 500, "currency": "EUR"}
        currency_request["final_total"] = {"minor": 500, "currency": "EUR"}
        currency = client.create_purchase_intent(currency_request)
        hostile_request = purchase("demo-hostile-form")
        hostile_request["payment_form"] = "merchant_controlled"
        hostile = client.create_purchase_intent(hostile_request)
        run("stop", "--data-dir", str(directory), "--owner-token-file", str(owner_file))
        stopped = client.create_purchase_intent(purchase("demo-stopped"))
        audit = run("audit", "--data-dir", str(directory), "--owner-token-file", str(owner_file))
        actions = [entry["action"] for entry in audit["entries"]]
        provider_charge_count = sum(
            entry["action"] == "provider_outcome"
            and entry.get("intent_id") == valid["id"]
            and entry.get("decision") == "settled"
            for entry in audit["entries"]
        )
        write_artifact(
            artifacts / "protocol-results.json",
            {
                "mcp": mcp,
                "starting_budget": starting_budget,
                "receiving": receiving,
                "settled": settled,
                "receipt": receipt,
                "remaining_budget": remaining_budget,
                "duplicate": duplicate,
                "duplicate_execution": duplicate_execution,
                "after_duplicate": after_duplicate,
                "adversarial_results": [over_budget, recurring, currency, hostile, stopped],
                "audit": audit,
            },
        )
        canary_result = subprocess.run(
            [str(BINARY), "demo"], cwd=ROOT, check=True, capture_output=True, text=True
        )
        (artifacts / "secret-provider.stdout").write_text(
            canary_result.stdout, encoding="utf-8"
        )
        (artifacts / "secret-provider.stderr").write_text(
            canary_result.stderr, encoding="utf-8"
        )
        canary_report = json.loads(canary_result.stdout)
        stop_processes()
        scan = subprocess.run(
            [
                sys.executable,
                str(ROOT / "scripts" / "secret-canary-scan.py"),
                str(artifacts),
                "--require",
                "daemon.log",
                "--require",
                "dashboard.log",
                "--require",
                "merchant.log",
                "--require",
                "mcp.stdout",
                "--require",
                "mcp.stderr",
                "--require",
                "protocol-results.json",
                "--require",
                "secret-provider.stdout",
                "--require",
                "secret-provider.stderr",
            ],
            cwd=ROOT, check=False, capture_output=True, text=True,
        )
        scan_clean = scan.returncode == 0
        if not scan_clean:
            raise RuntimeError("secret canary scan failed; inspect controlled artifacts")
        report = {
            "demo": "passed",
            "components": {"daemon": True, "dashboard": True, "mcp": True, "simulated_provider": True, "test_merchant": True},
            "owner_created": True,
            "agent_id": created["agent_id"],
            "mcp_status": mcp["status"],
            "starting_budget": starting_budget,
            "receiving_instructions": receiving,
            "valid_purchase": settled,
            "reservation_audited": "reserve_funds" in actions,
            "settlement_audited": "provider_outcome" in actions,
            "receipt": receipt,
            "remaining_budget": remaining_budget,
            "duplicate_intent_same_id": duplicate["id"] == valid["id"],
            "duplicate_execution_rejected": duplicate_execution["rejected"],
            "provider_charge_count_after_duplicate": provider_charge_count,
            "over_budget": over_budget,
            "recurring": recurring,
            "currency_substitution": currency,
            "merchant_controlled_form": hostile,
            "emergency_stop": stopped,
            "audit_chain": "valid" if audit["chain_valid"] else "invalid",
            "secret_canary": {
                "injected_through_secret_provider": canary_report["secret_canary"][
                    "volatile_secret_consumed_and_cleared"
                ],
                "full_pan_or_cvv_emitted": not scan_clean,
                "scan": "passed",
            },
        }
        print(json.dumps(report, indent=2))
    finally:
        stop_processes()
