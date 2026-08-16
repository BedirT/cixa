#!/usr/bin/env python3
"""Run available lockfile and vulnerability checks without hiding missing tools."""

from __future__ import annotations

import json
import re
import shutil
import subprocess
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
subprocess.run(["cargo", "metadata", "--locked", "--format-version", "1", "--no-deps"], cwd=ROOT, check=True, stdout=subprocess.DEVNULL)
audit = subprocess.run(["npm", "audit", "--audit-level=high", "--json"], cwd=ROOT, check=False, capture_output=True, text=True)
if audit.returncode != 0:
    try:
        report = json.loads(audit.stdout)
        counts = report.get("metadata", {}).get("vulnerabilities", {})
    except json.JSONDecodeError:
        counts = {"raw": audit.stdout[-500:]}
    raise SystemExit(f"npm audit reported high-severity findings: {counts}")

if not shutil.which("cargo-audit"):
    raise SystemExit("cargo-audit 0.22.2 is required; install it with: cargo install cargo-audit --version 0.22.2 --locked")
version_output = subprocess.run(
    ["cargo", "audit", "--version"], cwd=ROOT, check=True, capture_output=True, text=True
).stdout.strip()
version_match = re.fullmatch(r"cargo-audit(?:-audit)? ([0-9]+\.[0-9]+\.[0-9]+)", version_output)
if version_match is None or version_match.group(1) != "0.22.2":
    raise SystemExit(f"cargo-audit 0.22.2 is required, found: {version_output}")
for near_match in ("cargo-audit 0.22.20", "cargo-audit 10.22.2", "cargo-audit 0.22.2-beta"):
    if re.fullmatch(r"cargo-audit(?:-audit)? 0\.22\.2", near_match):
        raise SystemExit("internal cargo-audit version matcher accepted a near match")
for lockfile in ("Cargo.lock", "fuzz/Cargo.lock"):
    subprocess.run(["cargo", "audit", "--file", lockfile], cwd=ROOT, check=True)
print("dependency checks passed")
