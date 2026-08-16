#!/usr/bin/env python3
"""Build, inspect, install, and import the publishable npm workspace tarballs."""

from __future__ import annotations

import json
import subprocess
import tarfile
import tempfile
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]

with tempfile.TemporaryDirectory(prefix="agent-treasury-packages-") as raw_directory:
    directory = Path(raw_directory)
    tarballs: list[Path] = []
    for workspace in ("packages/sdk-typescript", "packages/mcp-server"):
        result = subprocess.run(
            ["npm", "pack", "--json", "--workspace", workspace, "--pack-destination", str(directory)],
            cwd=ROOT,
            check=True,
            capture_output=True,
            text=True,
        )
        tarball = directory / json.loads(result.stdout)[0]["filename"]
        tarballs.append(tarball)
        with tarfile.open(tarball, "r:gz") as archive:
            names = set(archive.getnames())
        required = {"package/LICENSE", "package/package.json", "package/dist/index.js"}
        if workspace.endswith("sdk-typescript"):
            required.add("package/dist/index.d.ts")
        missing = required - names
        if missing:
            raise SystemExit(f"{workspace} tarball is missing: {sorted(missing)}")

    install = directory / "install"
    install.mkdir()
    subprocess.run(["npm", "init", "-y"], cwd=install, check=True, stdout=subprocess.DEVNULL)
    subprocess.run(
        [
            "npm",
            "install",
            "--ignore-scripts",
            "--no-audit",
            "--no-fund",
            *(str(tarball) for tarball in tarballs),
        ],
        cwd=install,
        check=True,
        stdout=subprocess.DEVNULL,
    )
    subprocess.run(
        ["node", "--input-type=module", "-e", "await import('agent-treasury-sdk')"],
        cwd=install,
        check=True,
    )

print("npm package tarball assertions passed")
