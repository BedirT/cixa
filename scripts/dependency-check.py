#!/usr/bin/env python3
"""Run available lockfile and vulnerability checks without hiding missing tools."""

from __future__ import annotations

import json
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
version = subprocess.run(
    ["cargo", "audit", "--version"], cwd=ROOT, check=True, capture_output=True, text=True
).stdout
if "0.22.2" not in version:
    raise SystemExit(f"cargo-audit 0.22.2 is required, found: {version.strip()}")
subprocess.run(["cargo", "audit", "--file", "Cargo.lock"], cwd=ROOT, check=True)
print("dependency checks passed")
