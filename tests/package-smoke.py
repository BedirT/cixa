#!/usr/bin/env python3
"""Build, inspect, install, and import the publishable npm workspace tarballs."""

from __future__ import annotations

import json
import shutil
import subprocess
import tarfile
import tempfile
import zipfile
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]

build_version = subprocess.run(
    ["python3", "-c", "import importlib.metadata; print(importlib.metadata.version('build'))"],
    check=True,
    capture_output=True,
    text=True,
).stdout.strip()
if build_version != "1.4.2":
    raise SystemExit(f"Python build 1.4.2 is required, found {build_version}")

subprocess.run(
    [
        "cargo",
        "package",
        "--locked",
        "--no-verify",
        "--allow-dirty",
        "-p",
        "agent-treasury-domain",
    ],
    cwd=ROOT,
    check=True,
    stdout=subprocess.DEVNULL,
)
daemon_files = subprocess.run(
    ["cargo", "package", "--allow-dirty", "-p", "agent-treasury", "--list"],
    cwd=ROOT,
    check=True,
    capture_output=True,
    text=True,
).stdout.splitlines()
if not {"Cargo.toml", "src/main.rs"}.issubset(daemon_files):
    raise SystemExit("Rust daemon source package is incomplete")

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
    mcp_command = install / "node_modules" / ".bin" / "agent-treasury-mcp"
    mcp_result = subprocess.run(
        [str(mcp_command)], cwd=install, capture_output=True, text=True, timeout=10
    )
    if mcp_result.returncode != 2 or "TREASURY_SOCKET_PATH" not in mcp_result.stderr:
        raise SystemExit("installed MCP executable did not run under Node as expected")

    python_dist = directory / "python-dist"
    subprocess.run(
        ["python3", "-m", "build", "--outdir", str(python_dist), "packages/sdk-python"],
        cwd=ROOT,
        check=True,
        stdout=subprocess.DEVNULL,
    )
    wheel = next(python_dist.glob("*.whl"))
    source = next(python_dist.glob("*.tar.gz"))
    with zipfile.ZipFile(wheel) as archive:
        if not any(name.endswith("/licenses/LICENSE") for name in archive.namelist()):
            raise SystemExit("Python wheel is missing the Apache license")
    with tarfile.open(source, "r:gz") as archive:
        if not any(name.endswith("/LICENSE") for name in archive.getnames()):
            raise SystemExit("Python sdist is missing the Apache license")
    virtualenv = directory / "venv"
    subprocess.run(["python3", "-m", "venv", str(virtualenv)], check=True)
    subprocess.run(
        [str(virtualenv / "bin" / "pip"), "install", "--no-deps", str(wheel)],
        check=True,
        stdout=subprocess.DEVNULL,
    )
    subprocess.run(
        [str(virtualenv / "bin" / "python"), "-c", "import agent_treasury"], check=True
    )

shutil.rmtree(ROOT / "packages" / "sdk-python" / "agent_treasury_sdk.egg-info", ignore_errors=True)
print("npm package tarball assertions passed")
