#!/usr/bin/env python3
"""Fail-closed scan of generated runtime artifacts for synthetic payment canaries."""

import argparse
from pathlib import Path
import re


parser = argparse.ArgumentParser()
parser.add_argument("root", nargs="?", default="build")
parser.add_argument("--require", action="append", default=[])
args = parser.parse_args()
root = Path(args.root)
canaries = ("4111111111111111", "4111 1111 1111 1111", "4111-1111-1111-1111", "4111111111111111737")
cvv_pattern = re.compile(
    r'''(?ix)
    ["']?\b(?:cvv2?|cvc2?|security[ _-]?code)\b["']?
    \s*[:=]\s*["']?737["']?
    '''
)
violations: list[str] = []
if not root.is_dir():
    raise SystemExit(f"secret-canary scan root is missing or not a directory: {root}")
for required in args.require:
    required_path = root / required
    if not required_path.is_file():
        raise SystemExit(f"required secret-canary artifact is missing: {required_path}")
files = [path for path in root.rglob("*") if path.is_file()]
if not files:
    raise SystemExit(f"secret-canary scan found no artifacts: {root}")
for path in files:
    try:
        content = path.read_text(encoding="utf-8", errors="replace")
    except OSError as error:
        raise SystemExit(f"cannot read secret-canary artifact {path}: {error}") from error
    for canary in canaries:
        if canary in content:
            violations.append(f"{path}: synthetic PAN")
    if content.strip() == "737" or cvv_pattern.search(content):
        violations.append(f"{path}: synthetic CVV")
if violations:
    raise SystemExit("secret canary leaked:\n" + "\n".join(violations))
print(f"secret-canary scan passed ({root})")
