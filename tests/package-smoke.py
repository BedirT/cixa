#!/usr/bin/env python3
"""Build, inspect, install, and import the publishable npm workspace tarballs."""

from __future__ import annotations

import json
import os
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
for package, expected in {
    "setuptools": "83.0.0",
    "packaging": "26.0",
    "pyproject_hooks": "1.2.0",
}.items():
    installed = subprocess.run(
        [
            "python3",
            "-c",
            f"import importlib.metadata; print(importlib.metadata.version('{package}'))",
        ],
        check=True,
        capture_output=True,
        text=True,
    ).stdout.strip()
    if installed != expected:
        raise SystemExit(f"{package} {expected} is required, found {installed}")

subprocess.run(
    [
        "cargo",
        "package",
        "--locked",
        "--allow-dirty",
        "-p",
        "cixa-domain",
    ],
    cwd=ROOT,
    check=True,
    stdout=subprocess.DEVNULL,
)
daemon_files = subprocess.run(
    ["cargo", "package", "--allow-dirty", "-p", "cixa", "--list"],
    cwd=ROOT,
    check=True,
    capture_output=True,
    text=True,
).stdout.splitlines()
if not {"Cargo.toml", "src/main.rs"}.issubset(daemon_files):
    raise SystemExit("Rust daemon source package is incomplete")

with tempfile.TemporaryDirectory(prefix="cixa-packages-") as raw_directory:
    directory = Path(raw_directory)
    skill_environment = dict(
        os.environ,
        CODEX_HOME=str(directory / "codex-home"),
        CLAUDE_HOME=str(directory / "claude-home"),
    )
    subprocess.run(
        [str(ROOT / "scripts" / "install-agent-skill"), "--target", "all"],
        cwd=ROOT,
        env=skill_environment,
        check=True,
        stdout=subprocess.DEVNULL,
    )
    for home in (directory / "codex-home", directory / "claude-home"):
        installed_skill = home / "skills" / "cixa-payments" / "SKILL.md"
        if "Never retry an ambiguous payment" not in installed_skill.read_text(encoding="utf-8"):
            raise SystemExit(f"installed agent skill is incomplete: {installed_skill}")
    refused = subprocess.run(
        [str(ROOT / "scripts" / "install-agent-skill"), "--target", "all"],
        cwd=ROOT,
        env=skill_environment,
        capture_output=True,
        text=True,
    )
    if refused.returncode == 0 or "refusing to replace" not in refused.stderr:
        raise SystemExit("agent skill installer replaced an existing skill without --force")

    rust_install = directory / "rust-install"
    subprocess.run(
        [
            "cargo",
            "install",
            "--path",
            "apps/daemon",
            "--locked",
            "--root",
            str(rust_install),
        ],
        cwd=ROOT,
        check=True,
        stdout=subprocess.DEVNULL,
    )
    installed_cixa = rust_install / "bin" / "cixa"
    subprocess.run([str(installed_cixa), "--help"], check=True, stdout=subprocess.DEVNULL)
    subprocess.run([str(installed_cixa), "demo"], check=True, stdout=subprocess.DEVNULL)

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
        ["node", "--input-type=module", "-e", "await import('cixa-sdk')"],
        cwd=install,
        check=True,
    )
    mcp_command = install / "node_modules" / ".bin" / "cixa-mcp"
    mcp_result = subprocess.run(
        [str(mcp_command)], cwd=install, capture_output=True, text=True, timeout=10
    )
    if mcp_result.returncode != 2 or "CIXA_SOCKET_PATH" not in mcp_result.stderr:
        raise SystemExit("installed MCP executable did not run under Node as expected")

    python_dist = directory / "python-dist"
    subprocess.run(
        [
            "python3",
            "-m",
            "build",
            "--no-isolation",
            "--outdir",
            str(python_dist),
            "packages/sdk-python",
        ],
        cwd=ROOT,
        check=True,
        stdout=subprocess.DEVNULL,
    )
    wheel = next(python_dist.glob("*.whl"))
    source = next(python_dist.glob("*.tar.gz"))
    with zipfile.ZipFile(wheel) as archive:
        license_files = [name for name in archive.namelist() if name.endswith("/licenses/LICENSE")]
        if not license_files or b"GNU AFFERO GENERAL PUBLIC LICENSE" not in archive.read(license_files[0]):
            raise SystemExit("Python wheel is missing the AGPLv3 license")
    with tarfile.open(source, "r:gz") as archive:
        license_files = [name for name in archive.getnames() if name.endswith("/LICENSE")]
        license_member = archive.extractfile(license_files[0]) if license_files else None
        if license_member is None or b"GNU AFFERO GENERAL PUBLIC LICENSE" not in license_member.read():
            raise SystemExit("Python sdist is missing the AGPLv3 license")
    virtualenv = directory / "venv"
    subprocess.run(["python3", "-m", "venv", str(virtualenv)], check=True)
    subprocess.run(
        [str(virtualenv / "bin" / "pip"), "install", "--no-deps", str(wheel)],
        check=True,
        stdout=subprocess.DEVNULL,
    )
    subprocess.run(
        [str(virtualenv / "bin" / "python"), "-c", "import cixa"], check=True
    )

shutil.rmtree(ROOT / "packages" / "sdk-python" / "cixa_sdk.egg-info", ignore_errors=True)
print("Rust, npm, and Python package smoke assertions passed")
