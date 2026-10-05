#!/usr/bin/env python3
"""Capture read-only source/build identity for an isolated native test run.

Does not open state or credentials. Run after the corresponding tests.
"""
import hashlib
import json
import platform
import sys
from pathlib import Path


root = Path(__file__).resolve().parent.parent
label = sys.argv[1]
if not label or not all(c.isalnum() or c == "-" for c in label):
    raise SystemExit("invalid snapshot label")
architecture = {"aarch64": "arm64", "x86_64": "x86_64"}.get(platform.machine())
if platform.system() != "Linux" or architecture is None:
    raise SystemExit("native Linux qualification host required")
patterns = [
    "Cargo.toml", "Cargo.lock", "src/**/*.rs", "crates/**/*.rs",
    "crates/**/Cargo.toml", "tests/**/*.rs", "vendor/oci-client/src/**/*.rs",
    "vendor/oci-client/Cargo.toml", "vendor/oci-client/LICENSE",
    "vendor/oci-client/ARTIFACTD_PATCH.md", "deploy/*", "licenses/*",
    "vendor/tar/src/**/*.rs", "vendor/tar/Cargo.toml", "vendor/tar/LICENSE-*",
    "vendor/tar/ARTIFACTD_PATCH.md", "examples/**/*.rs",
    "vendor/redb/src/**/*.rs", "vendor/redb/Cargo.toml", "vendor/redb/build.rs",
    "vendor/redb/LICENSE-*", "vendor/redb/ARTIFACTD_PATCH.md",
]
sources = sorted({p for pattern in patterns for p in root.glob(pattern)
                  if p.is_file() and not any(part.startswith("._") for part in p.parts)})
hashes = {str(p.relative_to(root)): hashlib.sha256(p.read_bytes()).hexdigest() for p in sources}
binaries = {
    name: hashlib.sha256((root / "target/debug" / name).read_bytes()).hexdigest()
    for name in ["apollo-artifactd", "apollo-artifactctl"]
}
destination = root / f"docs/{label}-{architecture}-snapshot.json"
destination.parent.mkdir(exist_ok=True)
destination.write_text(json.dumps({
    "uname": list(platform.uname()), "build_profile": "debug",
    "sources": hashes, "binaries": binaries,
}, indent=2) + "\n")
print(destination)
