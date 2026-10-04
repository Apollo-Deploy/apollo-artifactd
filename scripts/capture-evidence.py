#!/usr/bin/env python3
"""Record a native build snapshot and the latest isolated churn store, read-only."""
import hashlib
import json
import platform
import sqlite3
from pathlib import Path


root = Path(__file__).resolve().parent.parent
architecture = "arm64" if platform.machine() == "aarch64" else "x86_64"
patterns = ["Cargo.toml", "Cargo.lock", "src/**/*.rs", "crates/**/*.rs", "crates/**/Cargo.toml"]
sources = sorted({p for pattern in patterns for p in root.glob(pattern) if p.is_file()})
hashes = {str(p.relative_to(root)): hashlib.sha256(p.read_bytes()).hexdigest() for p in sources}
binaries = {
    name: hashlib.sha256((root / "target/release" / name).read_bytes()).hexdigest()
    for name in ["apollo-artifactd", "apollo-artifactctl"]
}
(root / f"docs/native-{architecture}-snapshot.json").write_text(
    json.dumps({"uname": list(platform.uname()), "sources": hashes, "binaries": binaries}, indent=2) + "\n"
)
stores = list(Path.home().glob(f"artifactd-churn-{'arm64' if architecture == 'arm64' else 'x86'}.*"))
store = max(stores, key=lambda p: p.stat().st_mtime_ns)
with sqlite3.connect((store / "state.sqlite").as_uri() + "?mode=ro", uri=True) as db:
    tables = ["blobs", "imports", "pins", "leases", "roots", "edges", "prepared", "gc", "operations"]
    rows = {table: db.execute(f"SELECT count(*) FROM {table}").fetchone()[0] for table in tables}
    integrity = db.execute("PRAGMA quick_check").fetchone()[0]
state = {
    "store": str(store),
    "files": {p.name: p.stat().st_size for p in store.iterdir() if p.is_file()},
    "rows": rows,
    "integrity": integrity,
    "directory_entries": {name: sum(1 for _ in (store / name).iterdir()) for name in ["blobs", "temp", "prepared"]},
}
(root / f"docs/native-{architecture}-state.json").write_text(json.dumps(state, indent=2) + "\n")
