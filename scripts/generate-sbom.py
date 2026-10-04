#!/usr/bin/env python3
"""Generate a lockfile-based CycloneDX inventory, including dev/target dependencies.

This describes the Cargo graph, not the contents of a linked release binary.
Vendored packages have a distinct local reference and explicit fork provenance.
"""
import hashlib
import json
import subprocess
from pathlib import Path
from urllib.parse import quote


root = Path(__file__).resolve().parent.parent
metadata = json.loads(subprocess.check_output(
    ["cargo", "metadata", "--locked", "--format-version", "1"], cwd=root
))
packages = metadata["packages"]
references = {
    p["id"]: f'{p["source"] or "local"}#{p["name"]}@{p["version"]}'
    for p in packages
}
assert len(set(references.values())) == len(references), "ambiguous package identity"


def component(package):
    result = {
        "type": "library",
        "bom-ref": references[package["id"]],
        "name": package["name"],
        "version": package["version"],
        "purl": f'pkg:cargo/{quote(package["name"])}@{quote(package["version"])}',
    }
    if package["license"]:
        result["licenses"] = [{"expression": package["license"].replace("/", " OR ")}]
    if package["name"] == "oci-client" and package["source"] is None:
        result["properties"] = [{
            "name": "artifactd:vendored-fork",
            "value": "vendor/oci-client/ARTIFACTD_PATCH.md",
        }]
    return result


root_id = metadata["resolve"]["root"]
root_package = next(p for p in packages if p["id"] == root_id)
document = {
    "bomFormat": "CycloneDX", "specVersion": "1.5", "version": 1,
    "metadata": {
        "component": component(root_package),
        "properties": [{
            "name": "artifactd:cargo-lock-sha256",
            "value": hashlib.sha256((root / "Cargo.lock").read_bytes()).hexdigest(),
        }, {"name": "artifactd:scope", "value": "all Cargo targets and dependency kinds"}],
    },
    "components": [component(p) for p in sorted(packages, key=lambda p: references[p["id"]])
                   if p["id"] != root_id],
    "dependencies": [{
        "ref": references[node["id"]],
        "dependsOn": sorted({references[dep["pkg"]] for dep in node["deps"]}),
    } for node in sorted(metadata["resolve"]["nodes"], key=lambda n: references[n["id"]])],
}
(root / "docs/sbom.cdx.json").write_text(json.dumps(document, indent=2) + "\n")
print(f'Wrote {len(document["components"])} components and {len(document["dependencies"])} graph nodes')
