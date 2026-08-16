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


def http_body(port: int) -> str:
    connection = http.client.HTTPConnection("127.0.0.1", port, timeout=5)
    connection.request("GET", "/")
    response = connection.getresponse()
    body = response.read().decode("utf-8")
    connection.close()
    if response.status != 200:
        raise RuntimeError("test merchant did not return HTTP 200")
    return body


def owner_rpc(socket_path: Path, token: str, operation: dict) -> dict:
    envelope = {
        "api_version": "v1",
        "request_id": f"demo-owner-{time.time_ns()}",
        "token": token,
        "operation": operation,
    }
    with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as channel:
        channel.connect(str(socket_path))
        channel.sendall((json.dumps(envelope, separators=(",", ":")) + "\n").encode())
        response = b""
        while b"\n" not in response:
            response += channel.recv(64 * 1024)
    decoded = json.loads(response.split(b"\n", 1)[0])
    if not decoded["ok"]:
        raise RuntimeError(decoded["error"])
    return decoded["data"]


def purchase(key: str, amount: int = 500) -> dict:
    return {
        "idempotency_key": key,
        "amount": {"minor": amount, "currency": "CAD"},
        "final_total": {"minor": amount, "currency": "CAD"},
        "merchant_domain": "merchant.example.test",
        "category": "software",
        "items": [{"label": "demo item", "quantity": 1, "unit_price_minor": amount}],
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
        owner_token = owner_file.read_text(encoding="utf-8").strip()
        overview = owner_rpc(owner_socket, owner_token, {"type": "owner_get_dashboard"})
        policy = next(iter(overview["policies"].values()))
        policy["max_transactions_per_minute"] = 100
        owner_rpc(
            owner_socket,
            owner_token,
            {"type": "owner_update_policy", "agent_id": created["agent_id"], "policy": policy},
        )
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
        merchant_fixture = http_body(merchant_port)
        for scenario_name in (
            "amount_changed",
            "currency_changed",
            "hidden_recurring",
            "card_saving",
            "trial_auto_renew",
            "preauthorization",
            "delayed_settlement",
            "decline",
            "timeout_before_submit",
            "timeout_after_submit",
            "duplicate_form_submission",
            "misleading_success_page",
            "cross_origin_fields",
            "merchant_controlled_form",
            "redirect_to_other_domain",
            "redirect_to_localhost",
            "dns_rebinding_like",
            "prompt_injection",
            "screenshot_and_trace_leak",
            "forged_deposit",
            "spoofed_receipt",
            "browser_crash",
        ):
            if scenario_name not in merchant_fixture:
                raise RuntimeError(f"test merchant is missing scenario {scenario_name}")
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

        hostile_checkout: dict[str, dict] = {}

        def proposed_case(name: str, **changes: object) -> dict:
            request = purchase(f"hostile-{name}", 100)
            request.update(changes)
            try:
                return client.create_purchase_intent(request)
            except BrokerError as error:
                return {"broker_rejected": True, "reason": str(error)}

        hostile_checkout["amount_increase"] = proposed_case(
            "amount-increase",
            final_total={"minor": 200, "currency": "CAD"},
            scenario="amount_changed",
        )
        hostile_checkout["hidden_recurring"] = proposed_case(
            "hidden-recurring", recurring=True, scenario="hidden_recurring"
        )
        hostile_checkout["trial_auto_renew"] = proposed_case(
            "trial-auto-renew", trial_auto_renew=True
        )
        hostile_checkout["card_saving"] = proposed_case(
            "card-saving", stored_card=True, scenario="card_saving"
        )
        hostile_checkout["tip"] = proposed_case("tip", tip_minor=25, scenario="tip")
        hostile_checkout["preauthorization"] = proposed_case(
            "preauthorization", preauthorization=True, scenario="preauthorization"
        )
        hostile_checkout["installments"] = proposed_case("installments", installments=True)
        hostile_checkout["cross_origin_fields"] = proposed_case(
            "cross-origin-fields",
            redirect_chain=["https://processor.example.test/hosted"],
        )
        hostile_checkout["redirect_other_domain"] = proposed_case(
            "redirect-other-domain",
            redirect_chain=["https://attacker.example.test/pay"],
            scenario="redirect_to_other_domain",
        )
        hostile_checkout["redirect_localhost"] = proposed_case(
            "redirect-localhost",
            redirect_chain=["https://127.0.0.1/pay"],
            scenario="redirect_to_localhost",
        )
        hostile_checkout["dns_rebinding"] = proposed_case(
            "dns-rebinding", scenario="dns_rebinding_like"
        )
        hostile_checkout["prompt_injection"] = proposed_case(
            "prompt-injection", scenario="prompt_injection"
        )
        hostile_checkout["malicious_field_inspection"] = proposed_case(
            "malicious-fields",
            payment_form="merchant_controlled",
            scenario="merchant_controlled_form",
        )
        hostile_checkout["screenshot_trace_attempt"] = proposed_case(
            "capture-attempt",
            payment_form="merchant_controlled",
            scenario="merchant_controlled_form",
        )

        for index, (name, scenario) in enumerate((
            ("delayed_settlement", "delayed_settlement"),
            ("decline", "decline"),
            ("timeout_before_submit", "timeout_before_submit"),
            ("timeout_after_submit", "timeout_after_submit"),
            ("misleading_success", "misleading_success_page"),
            ("browser_crash", "browser_crash"),
            ("duplicate_form_submission", "duplicate_form_submission"),
        )):
            case_amount = 120 + index
            intent = proposed_case(
                name,
                scenario=scenario,
                amount={"minor": case_amount, "currency": "CAD"},
                final_total={"minor": case_amount, "currency": "CAD"},
            )
            hostile_checkout[name] = {
                "intent": intent,
                "execution": (
                    client.execute_purchase_intent(intent["id"])
                    if intent.get("state") == "policy_validated"
                    else {"not_executable_state": intent.get("state", "broker_rejected")}
                ),
            }
            execution = hostile_checkout[name]["execution"]
            if execution.get("status") in {"provider_pending", "unknown", "settled"}:
                try:
                    client.execute_purchase_intent(intent["id"])
                    hostile_checkout[name]["retry_rejected"] = False
                except BrokerError as error:
                    hostile_checkout[name]["retry_rejected"] = str(error).startswith("conflict:")

        try:
            client.request(
                {
                    "type": "owner_record_deposit",
                    "amount": {"minor": 9999, "currency": "CAD"},
                    "source": "forged-agent-notification",
                    "verified": True,
                    "agent_id": created["agent_id"],
                    "external_reference": "forged-agent-deposit",
                }
            )
            hostile_checkout["forged_deposit"] = {"rejected": False}
        except BrokerError as error:
            hostile_checkout["forged_deposit"] = {
                "rejected": str(error) == "owner operations require the owner control socket"
            }
        try:
            client.get_receipt("spoofed-receipt-id")
            hostile_checkout["spoofed_receipt"] = {"rejected": False}
        except BrokerError as error:
            hostile_checkout["spoofed_receipt"] = {
                "rejected": str(error).startswith("not found:")
            }
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
                "hostile_checkout": hostile_checkout,
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
            "hostile_checkout": hostile_checkout,
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
