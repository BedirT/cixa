#!/usr/bin/env python3
"""Generate a small local SBOM from the locked Rust and Node dependency graphs."""

from __future__ import annotations

import json
import subprocess
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / "build" / "sbom.json"
OUT.parent.mkdir(exist_ok=True)
metadata = json.loads(subprocess.run(["cargo", "metadata", "--locked", "--format-version", "1"], cwd=ROOT, check=True, capture_output=True, text=True).stdout)
components = [
    {"type": "library", "name": package["name"], "version": package["version"], "ecosystem": "cargo"}
    for package in metadata["packages"]
]
lock = json.loads((ROOT / "package-lock.json").read_text(encoding="utf-8"))
for package_path, package in lock.get("packages", {}).items():
    if package_path and package.get("version"):
        components.append({"type": "library", "name": package_path.rsplit("/node_modules/", 1)[-1], "version": package["version"], "ecosystem": "npm"})
OUT.write_text(json.dumps({"bomFormat": "CycloneDX", "specVersion": "1.5", "components": components}, indent=2) + "\n", encoding="utf-8")
print(f"SBOM written to {OUT}")
