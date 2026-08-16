#!/usr/bin/env python3
"""Check every locked Rust and npm dependency using SPDX-expression semantics."""

from __future__ import annotations

import json
import re
import subprocess
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
metadata = json.loads(subprocess.run(["cargo", "metadata", "--locked", "--format-version", "1"], cwd=ROOT, check=True, capture_output=True, text=True).stdout)
allowed = {
    "MIT",
    "Apache-2.0",
    "BSD-2-Clause",
    "BSD-3-Clause",
    "ISC",
    "Zlib",
    "Unicode-3.0",
    "Unlicense",
    "LLVM-exception",
}


def permissive_expression(expression: str) -> bool:
    tokens = re.findall(r"\(|\)|AND|OR|WITH|[A-Za-z0-9.+-]+", expression.replace("/", " OR "))
    position = 0

    def factor() -> bool:
        nonlocal position
        if position >= len(tokens):
            raise ValueError("missing SPDX factor")
        if tokens[position] == "(":
            position += 1
            value = disjunction()
            if position >= len(tokens) or tokens[position] != ")":
                raise ValueError("unclosed SPDX expression")
            position += 1
            return value
        license_id = tokens[position]
        position += 1
        value = license_id in allowed
        if position < len(tokens) and tokens[position] == "WITH":
            position += 1
            if position >= len(tokens):
                raise ValueError("missing SPDX exception")
            value = value and tokens[position] in allowed
            position += 1
        return value

    def conjunction() -> bool:
        nonlocal position
        value = factor()
        while position < len(tokens) and tokens[position] == "AND":
            position += 1
            value = factor() and value
        return value

    def disjunction() -> bool:
        nonlocal position
        value = conjunction()
        while position < len(tokens) and tokens[position] == "OR":
            position += 1
            value = conjunction() or value
        return value

    result = disjunction()
    if position != len(tokens):
        raise ValueError("unsupported SPDX expression")
    return result


unknown = []
for package in metadata["packages"]:
    license_value = package.get("license") or ""
    if license_value and not permissive_expression(license_value):
        unknown.append(f"{package['name']}={license_value}")
if unknown:
    raise SystemExit("non-permissive or unknown Cargo licenses: " + ", ".join(unknown))

lockfile = json.loads((ROOT / "package-lock.json").read_text(encoding="utf-8"))
unknown_npm = []
for path, package in lockfile["packages"].items():
    if not path or package.get("link") is True:
        continue
    license_value = package.get("license") or ""
    if not license_value or not permissive_expression(license_value):
        unknown_npm.append(f"{path}={license_value or 'missing'}")
if unknown_npm:
    raise SystemExit("non-permissive or unknown npm licenses: " + ", ".join(unknown_npm))
print("license check passed for the locked Cargo and npm graphs")
