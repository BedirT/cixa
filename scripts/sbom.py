#!/usr/bin/env python3
"""Generate a validated CycloneDX 1.5 SBOM from all locked dependency graphs."""

from __future__ import annotations

import importlib.metadata
import json
import subprocess
import uuid
from pathlib import Path

from cyclonedx.model.bom import Bom
from cyclonedx.model.component import Component, ComponentType
from cyclonedx.output.json import JsonV1Dot5
from packageurl import PackageURL


ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / "build" / "sbom.json"
OUT.parent.mkdir(exist_ok=True)

EXPECTED_TOOLS = {
    "cyclonedx-python-lib": "9.1.0",
    "packageurl-python": "0.17.6",
}
for distribution, expected in EXPECTED_TOOLS.items():
    actual = importlib.metadata.version(distribution)
    if actual != expected:
        raise SystemExit(f"{distribution} {expected} is required for deterministic SBOM output")


def package_url(ecosystem: str, name: str, version: str) -> PackageURL:
    namespace = None
    package = name
    if ecosystem == "npm" and name.startswith("@"):
        namespace, package = name[1:].split("/", 1)
    return PackageURL(type=ecosystem, namespace=namespace, name=package, version=version)


components: list[Component] = []
metadata = json.loads(
    subprocess.run(
        ["cargo", "metadata", "--locked", "--format-version", "1"],
        cwd=ROOT,
        check=True,
        capture_output=True,
        text=True,
    ).stdout
)
for package in metadata["packages"]:
    component_type = (
        ComponentType.APPLICATION
        if any("bin" in target["kind"] for target in package["targets"])
        else ComponentType.LIBRARY
    )
    components.append(
        Component(
            type=component_type,
            name=package["name"],
            version=package["version"],
            purl=package_url("cargo", package["name"], package["version"]),
        )
    )

lock = json.loads((ROOT / "package-lock.json").read_text(encoding="utf-8"))
workspace_names: set[str] = set()
for package_path, package in lock.get("packages", {}).items():
    version = package.get("version")
    if not package_path or not version or package.get("link"):
        continue
    if package_path.startswith("packages/"):
        name = package.get("name")
        if not name:
            raise SystemExit(f"npm workspace package has no declared name: {package_path}")
        workspace_names.add(name)
        component_type = ComponentType.APPLICATION if package.get("bin") else ComponentType.LIBRARY
    elif package_path.startswith("node_modules/"):
        name = package_path.rsplit("/node_modules/", 1)[-1]
        if name.startswith("node_modules/"):
            name = name.removeprefix("node_modules/")
        component_type = ComponentType.LIBRARY
    else:
        raise SystemExit(f"unsupported npm lockfile package path: {package_path}")
    components.append(
        Component(
            type=component_type,
            name=name,
            version=version,
            purl=package_url("npm", name, version),
        )
    )

for line in (ROOT / "requirements-build.lock").read_text(encoding="utf-8").splitlines():
    if line and not line.startswith("#"):
        requirement = line.split(" ", 1)[0]
        name, version = requirement.split("==", 1)
        components.append(
            Component(
                type=ComponentType.LIBRARY,
                name=name,
                version=version,
                purl=package_url("pypi", name, version),
            )
        )

expected_workspaces = {
    "agent-treasury-checkout-playwright",
    "agent-treasury-mcp-server",
    "agent-treasury-sdk",
}
if workspace_names != expected_workspaces:
    raise SystemExit(f"unexpected npm workspace inventory: {sorted(workspace_names)}")

purls = sorted(str(component.purl) for component in components)
bom = Bom(
    components=components,
    serial_number=uuid.uuid5(uuid.NAMESPACE_URL, "\n".join(purls)),
    version=1,
)
document = JsonV1Dot5(bom).output_as_string(indent=2)
decoded = json.loads(document)
if decoded.get("bomFormat") != "CycloneDX" or decoded.get("specVersion") != "1.5":
    raise SystemExit("CycloneDX serializer produced an unexpected document version")
for component in decoded.get("components", []):
    parsed = PackageURL.from_string(component["purl"])
    parsed_name = f"@{parsed.namespace}/{parsed.name}" if parsed.namespace else parsed.name
    if parsed.type == "npm" and parsed_name != component["name"]:
        raise SystemExit(f"component PURL identity mismatch: {component['name']}")

OUT.write_text(document + "\n", encoding="utf-8")
print(f"SBOM written to {OUT}")
