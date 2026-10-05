#!/usr/bin/env python3
"""Validate the inventory with an independently downloaded official schema.

Usage: validate-sbom.py /path/to/CycloneDX-1.5-schema-directory
Requires jsonschema and referencing; neither is a production dependency.
"""
import hashlib
import json
import sys
from pathlib import Path

from jsonschema import Draft7Validator
from referencing import Registry, Resource


root = Path(__file__).resolve().parent.parent
directory = Path(sys.argv[1])
names = ["bom-1.5.schema.json", "spdx.schema.json", "jsf-0.82.schema.json"]
schemas = {name: json.loads((directory / name).read_text()) for name in names}
registry = Registry()
base = "https://raw.githubusercontent.com/CycloneDX/specification/1.5/schema/"
for name, schema in schemas.items():
    resource = Resource.from_contents(schema)
    for prefix in [base, "http://cyclonedx.org/schema/", "https://cyclonedx.org/schema/"]:
        registry = registry.with_resource(prefix + name, resource)
doc = json.loads((root / "docs/sbom.cdx.json").read_text())
Draft7Validator(schemas[names[0]], registry=registry).validate(doc)
references = {c["bom-ref"] for c in doc["components"]}
references.add(doc["metadata"]["component"]["bom-ref"])
assert len(references) == len(doc["components"]) + 1, "duplicate component identity"
nodes = {node["ref"] for node in doc["dependencies"]}
assert nodes == references, "incomplete dependency nodes"
assert len(nodes) == len(doc["dependencies"]), "duplicate dependency node"
for node in doc["dependencies"]:
    assert set(node["dependsOn"]) <= references, "unresolved dependency reference"
lock_hash = hashlib.sha256((root / "Cargo.lock").read_bytes()).hexdigest()
assert doc["metadata"]["properties"][0]["value"] == lock_hash, "stale lockfile inventory"
print("PASS: official CycloneDX 1.5 schema, unique references and complete dependency edges")
print("Schema source: " + base + names[0])
print("Components: " + str(len(doc["components"])))
print("Lockfile SHA-256: " + lock_hash)
