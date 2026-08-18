#!/usr/bin/env python3
"""Validate the security-relevant Compose boundary without starting Docker."""

from __future__ import annotations

import json
import subprocess
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
completed = subprocess.run(
    [
        "docker",
        "compose",
        "--project-directory",
        str(ROOT),
        "--profile",
        "agent",
        "config",
        "--format",
        "json",
    ],
    check=True,
    capture_output=True,
    text=True,
)
configuration = json.loads(completed.stdout)
services = configuration["services"]
assert set(services) == {"cixa-init", "cixa-broker", "cixa-console", "cixa-mcp"}

initializer = services["cixa-init"]
assert initializer["user"] == "0:0"
assert initializer["network_mode"] == "none"
assert set(initializer["cap_add"]) == {"CHOWN", "DAC_OVERRIDE", "FOWNER"}

broker = services["cixa-broker"]
console = services["cixa-console"]
mcp = services["cixa-mcp"]
for owner_service in (broker, console):
    assert owner_service["user"] == "10000:10000"
    assert owner_service["read_only"] is True
    assert owner_service["cap_drop"] == ["ALL"]
    assert "no-new-privileges:true" in owner_service["security_opt"]
    assert set(owner_service["group_add"]) == {"12000"}
    targets = {mount["target"] for mount in owner_service["volumes"]}
    assert targets == {"/var/lib/cixa", "/run/cixa-agent"}

published = console["ports"]
assert len(published) == 1
assert published[0]["host_ip"] == "127.0.0.1"
assert published[0]["target"] == 8765

assert mcp["user"] == "10001:10001"
assert mcp["network_mode"] == "none"
assert mcp["read_only"] is True
assert mcp["cap_drop"] == ["ALL"]
assert set(mcp["group_add"]) == {"12000"}
assert mcp["environment"]["CIXA_SOCKET_PATH"] == "/run/cixa-agent/cixa.sock"
assert mcp["environment"]["CIXA_AGENT_TOKEN_FILE"] == "/run/cixa-agent/tokens/default.token"
assert len(mcp["volumes"]) == 1
assert mcp["volumes"][0]["target"] == "/run/cixa-agent"
assert mcp["volumes"][0]["read_only"] is True
assert "/var/lib/cixa" not in {mount["target"] for mount in mcp["volumes"]}

dockerfile = (ROOT / "Dockerfile").read_text(encoding="utf-8")
assert "FROM runtime-base AS agent" in dockerfile
assert "FROM runtime-base AS owner" in dockerfile
assert "COPY --from=rust-builder" in dockerfile
assert "install --with-deps chromium" in dockerfile

compose = (ROOT / "compose.yaml").read_text(encoding="utf-8")
assert "/var/run/docker.sock" not in compose
assert "CIXA_AGENT_TOKEN_FILE" in compose
assert "owner.token:" not in compose

print("Docker Compose security-boundary assertions passed")
