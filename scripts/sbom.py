#!/usr/bin/env python3
"""Generate a small local SBOM from the locked Rust and Node dependency graphs."""

from __future__ import annotations

import json
import subprocess
from pathlib import Path
from urllib.parse import quote


ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / "build" / "sbom.json"
OUT.parent.mkdir(exist_ok=True)


def npm_purl(name: str, version: str) -> str:
    if name.startswith("@") and "/" in name:
        namespace, package = name.split("/", 1)
        return f"pkg:npm/{quote(namespace, safe='')}/{quote(package, safe='')}@{quote(version, safe='')}"
    return f"pkg:npm/{quote(name, safe='')}@{quote(version, safe='')}"


metadata = json.loads(subprocess.run(["cargo", "metadata", "--locked", "--format-version", "1"], cwd=ROOT, check=True, capture_output=True, text=True).stdout)
components = [
    {
        "type": "library",
        "name": package["name"],
        "version": package["version"],
        "purl": f"pkg:cargo/{quote(package['name'], safe='')}@{quote(package['version'], safe='')}",
    }
    for package in metadata["packages"]
]
lock = json.loads((ROOT / "package-lock.json").read_text(encoding="utf-8"))
for package_path, package in lock.get("packages", {}).items():
    if package_path and package.get("version"):
        name = package_path.rsplit("/node_modules/", 1)[-1]
        components.append({
            "type": "library",
            "name": name,
            "version": package["version"],
            "purl": npm_purl(name, package["version"]),
        })
for line in (ROOT / "requirements-build.lock").read_text(encoding="utf-8").splitlines():
    if line and not line.startswith("#"):
        requirement = line.split(" ", 1)[0]
        name, version = requirement.split("==", 1)
        components.append(
            {
                "type": "library",
                "name": name,
                "version": version,
                "purl": f"pkg:pypi/{quote(name, safe='')}@{quote(version, safe='')}",
            }
        )
document = {
    "bomFormat": "CycloneDX",
    "specVersion": "1.5",
    "version": 1,
    "components": components,
}
assert isinstance(document["version"], int) and document["version"] >= 1
assert all(
    component["type"] == "library"
    and component["name"]
    and component["version"]
    and component["purl"].startswith("pkg:")
    and set(component) <= {"type", "name", "version", "purl"}
    for component in components
)
OUT.write_text(json.dumps(document, indent=2) + "\n", encoding="utf-8")
print(f"SBOM written to {OUT}")
