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
    ["']?\b(?:cvv2?|cvc2?|security[ _-]?code|card[ _-]?verification[ _-]?value)\b["']?
    \s*(?:[:=]\s*)?["']?737["']?
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
        encoded = path.read_bytes()
    except OSError as error:
        raise SystemExit(f"cannot read secret-canary artifact {path}: {error}") from error
    for canary in canaries:
        if canary.encode("ascii") in encoded:
            violations.append(f"{path}: synthetic PAN")
    # Latin-1 preserves every byte, allowing the ASCII-oriented expression to
    # inspect mixed or binary artifacts without treating valid binary as an error.
    content = encoded.decode("latin-1")
    if content.strip() == "737" or cvv_pattern.search(content):
        violations.append(f"{path}: synthetic CVV")
if violations:
    raise SystemExit("secret canary leaked:\n" + "\n".join(violations))
print(f"secret-canary scan passed ({root})")
