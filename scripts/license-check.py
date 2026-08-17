#!/usr/bin/env python3
"""Check every locked Rust and npm dependency using SPDX-expression semantics."""

from __future__ import annotations

import json
import re
import subprocess
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
allowed = {
    "MIT",
    "NCSA",
    "PSF-2.0",
    "Apache-2.0",
    "BSD-2-Clause",
    "BSD-3-Clause",
    "ISC",
    "Zlib",
    "Unicode-3.0",
    "Unlicense",
}
allowed_exceptions = {"LLVM-exception"}
allowed_with_pairs = {("Apache-2.0", "LLVM-exception")}


def permissive_expression(expression: str) -> bool:
    normalized = expression.replace("/", " OR ")
    token_pattern = re.compile(r"\s*(\(|\)|AND|OR|WITH|[A-Za-z0-9.+-]+)")
    tokens: list[str] = []
    cursor = 0
    while cursor < len(normalized):
        match = token_pattern.match(normalized, cursor)
        if match is None:
            raise ValueError(f"invalid SPDX syntax at offset {cursor}")
        tokens.append(match.group(1))
        cursor = match.end()
    if not tokens:
        raise ValueError("empty SPDX expression")
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
            exception_id = tokens[position]
            value = value and exception_id in allowed_exceptions and (
                license_id,
                exception_id,
            ) in allowed_with_pairs
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


try:
    permissive_expression("MIT?")
except ValueError:
    pass
else:
    raise SystemExit("internal SPDX parser accepted malformed syntax")
for invalid_expression in ("LLVM-exception", "MIT WITH MIT"):
    if permissive_expression(invalid_expression):
        raise SystemExit(f"internal SPDX parser accepted invalid expression: {invalid_expression}")

unknown = []
seen_cargo = set()
for manifest in ("Cargo.toml", "fuzz/Cargo.toml"):
    metadata = json.loads(
        subprocess.run(
            ["cargo", "metadata", "--locked", "--format-version", "1", "--manifest-path", manifest],
            cwd=ROOT,
            check=True,
            capture_output=True,
            text=True,
        ).stdout
    )
    for package in metadata["packages"]:
        identity = (package["name"], package["version"], package["source"])
        if identity in seen_cargo:
            continue
        seen_cargo.add(identity)
        license_value = package.get("license") or ""
        if not license_value or not permissive_expression(license_value):
            unknown.append(f"{package['name']}={license_value or 'missing'}")
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

python_lock = (ROOT / "requirements-build.lock").read_text(encoding="utf-8").splitlines()
for line in python_lock:
    if not line or line.startswith("#"):
        continue
    if "# license: " not in line:
        raise SystemExit(f"Python build dependency lacks reviewed license evidence: {line}")
    expression = line.split("# license: ", 1)[1]
    if not permissive_expression(expression):
        raise SystemExit(f"non-permissive Python build license: {expression}")
print("license check passed for the locked Cargo, npm, and Python build graphs")
