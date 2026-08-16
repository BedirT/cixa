#!/usr/bin/env python3
"""Assert the local demo handles the required adversarial checkout outcomes."""

from __future__ import annotations

import json
import os
import subprocess
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
result = subprocess.run(
    [str(ROOT / "scripts" / "demo")],
    cwd=ROOT,
    check=True,
    capture_output=True,
    text=True,
)
if "4111111111111111" in result.stdout or "737" in result.stdout:
    raise SystemExit("demo emitted a synthetic secret")
report = json.loads(result.stdout)
assert report["demo"] == "passed"
assert report["provider_charge_count_after_duplicate"] == 1
assert report["duplicate_intent_same_id"] is True
assert all(report["components"].values())
assert report["audit_chain"] == "valid"
assert report["secret_canary"]["full_pan_or_cvv_emitted"] is False
for name in ("over_budget", "recurring", "currency_substitution", "emergency_stop"):
    assert report[name]["state"] == "failed", (name, report[name])
assert report["merchant_controlled_form"]["state"] == "approval_required"
print("adversarial demo assertions passed")
