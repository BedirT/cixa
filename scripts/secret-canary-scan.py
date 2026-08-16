#!/usr/bin/env python3
"""Scan generated runtime artifacts for the synthetic payment canary only."""

from pathlib import Path
import sys


root = Path(sys.argv[1] if len(sys.argv) > 1 else "build")
canaries = ("4111111111111111", "4111 1111 1111 1111", "4111-1111-1111-1111", "4111111111111111737")
violations: list[str] = []
if root.exists():
    for path in root.rglob("*"):
        if path.is_file():
            try:
                content = path.read_text(encoding="utf-8", errors="replace")
            except OSError:
                continue
            for canary in canaries:
                if canary in content:
                    violations.append(f"{path}: {canary}")
if violations:
    raise SystemExit("secret canary leaked:\n" + "\n".join(violations))
print(f"secret-canary scan passed ({root})")
