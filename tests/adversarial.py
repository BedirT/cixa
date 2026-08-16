#!/usr/bin/env python3
"""Assert the local demo handles the required adversarial checkout outcomes."""

from __future__ import annotations

import json
import os
import subprocess
import tempfile
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
result = subprocess.run(
    [str(ROOT / "scripts" / "demo")],
    cwd=ROOT,
    check=True,
    capture_output=True,
    text=True,
)
if "4111111111111111" in result.stdout:
    raise SystemExit("demo emitted a synthetic secret")
report = json.loads(result.stdout)
assert report["demo"] == "passed"
assert report["provider_charge_count_after_duplicate"] == 1
assert report["duplicate_intent_same_id"] is True
assert report["duplicate_execution_rejected"] is True
assert all(report["components"].values())
assert report["audit_chain"] == "valid"
assert report["secret_canary"]["full_pan_or_cvv_emitted"] is False
assert report["secret_canary"]["injected_through_secret_provider"] is True
assert report["secret_canary"]["scan"] == "passed"
hostile = report["hostile_checkout"]
for name in ("amount_increase", "redirect_localhost"):
    assert hostile[name]["broker_rejected"] is True, (name, hostile[name])
for name in (
    "hidden_recurring",
    "trial_auto_renew",
    "card_saving",
    "tip",
    "preauthorization",
    "installments",
    "cross_origin_fields",
    "redirect_other_domain",
):
    assert hostile[name]["state"] == "failed", (name, hostile[name])
for name in (
    "dns_rebinding",
    "prompt_injection",
    "malicious_field_inspection",
    "screenshot_trace_attempt",
):
    assert hostile[name]["state"] == "approval_required", (name, hostile[name])
assert hostile["delayed_settlement"]["execution"]["status"] == "provider_pending"
assert hostile["decline"]["execution"]["status"] == "declined"
for name in ("timeout_before_submit", "timeout_after_submit", "browser_crash"):
    assert hostile[name]["execution"]["status"] == "unknown", (name, hostile[name])
    assert hostile[name]["retry_rejected"] is True
for name in ("misleading_success", "duplicate_form_submission"):
    assert hostile[name]["execution"]["status"] == "settled", (name, hostile[name])
    assert hostile[name]["retry_rejected"] is True
assert hostile["delayed_settlement"]["retry_rejected"] is True
assert hostile["forged_deposit"]["rejected"] is True
assert hostile["spoofed_receipt"]["rejected"] is True
for leaked_value in (
    "checkout error: cvv=737\n",
    "checkout error: CVV 737\n",
    '{"cvv":"737"}\n',
    '{\n  "cvc": "737"\n}\n',
    '{"security_code":"737"}\n',
    '{"card_verification_value":"737"}\n',
):
    with tempfile.TemporaryDirectory(prefix="agent-treasury-cvv-regression-") as directory:
        Path(directory, "controlled.log").write_text(leaked_value, encoding="utf-8")
        cvv_scan = subprocess.run(
            ["python3", str(ROOT / "scripts" / "secret-canary-scan.py"), directory],
            cwd=ROOT,
            capture_output=True,
            text=True,
        )
        assert cvv_scan.returncode != 0
missing_scan = subprocess.run(
    ["python3", str(ROOT / "scripts" / "secret-canary-scan.py"), "/definitely/missing"],
    cwd=ROOT,
    capture_output=True,
    text=True,
)
assert missing_scan.returncode != 0
with tempfile.TemporaryDirectory(prefix="agent-treasury-binary-regression-") as directory:
    Path(directory, "capture.png").write_bytes(b"\x89PNG\r\n\x1a\n\x00\xff")
    binary_scan = subprocess.run(
        ["python3", str(ROOT / "scripts" / "secret-canary-scan.py"), directory],
        cwd=ROOT,
        capture_output=True,
        text=True,
    )
    assert binary_scan.returncode != 0
for name in ("over_budget", "recurring", "currency_substitution", "emergency_stop"):
    assert report[name]["state"] == "failed", (name, report[name])
assert report["merchant_controlled_form"]["state"] == "approval_required"
print("adversarial demo assertions passed")
