#!/usr/bin/env python3
"""Fail closed when the documented security and release surfaces are missing."""

from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
REQUIRED = [
    "README.md",
    "PLAN.md",
    "PROGRESS.md",
    "THREAT_MODEL.md",
    "SECURITY.md",
    "CONTRIBUTING.md",
    "LICENSE",
    "Dockerfile",
    "compose.yaml",
    "docs/research.md",
    "docs/architecture.md",
    "docs/security-model.md",
    "docs/credential-handling.md",
    "docs/agent-integration.md",
    "docs/provider-adapters.md",
    "docs/checkout-adapters.md",
    "docs/limitations.md",
    "docs/incident-response.md",
    "docs/coverage.md",
    "docs/deployment.md",
    "docs/docker.md",
    "docs/koho-setup.md",
    "docs/owner-console-plan.md",
    "skills/cixa-payments/SKILL.md",
    "skills/cixa-payments/references/purchase-contract.md",
    "skills/cixa-payments/references/state-guide.md",
    "docs/adr/0001-core-boundary.md",
    "docs/adr/0002-secret-handling.md",
    "docs/adr/0003-checkout-trust.md",
    "docs/assets/cixa-architecture.svg",
]

missing = [path for path in REQUIRED if not (ROOT / path).is_file()]
if missing:
    raise SystemExit("missing documentation: " + ", ".join(missing))

readme = (ROOT / "README.md").read_text(encoding="utf-8").lower()
for phrase in ("not a bank", "does not use a koho api", "pci", "emergency stop", "no real transaction"):
    if phrase not in readme:
        raise SystemExit(f"README is missing required phrase: {phrase}")

for path in REQUIRED:
    text = (ROOT / path).read_text(encoding="utf-8")
    if "PLACEHOLDER" in text or "lorem ipsum" in text.lower():
        raise SystemExit(f"placeholder text found in {path}")

print(f"documentation check passed ({len(REQUIRED)} required files)")
