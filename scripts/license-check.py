#!/usr/bin/env python3
"""Check that the direct project dependencies have permissive declared licenses."""

from __future__ import annotations

import json
import subprocess
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
metadata = json.loads(subprocess.run(["cargo", "metadata", "--locked", "--format-version", "1"], cwd=ROOT, check=True, capture_output=True, text=True).stdout)
allowed = {"MIT", "Apache-2.0", "BSD-2-Clause", "BSD-3-Clause", "ISC", "Zlib", "Unicode-3.0"}
unknown = []
for package in metadata["packages"]:
    license_value = package.get("license") or ""
    if license_value and not any(item in license_value for item in allowed):
        unknown.append(f"{package['name']}={license_value}")
if unknown:
    raise SystemExit("non-permissive or unknown Cargo licenses: " + ", ".join(unknown))
print("license check passed for the locked Cargo graph")
