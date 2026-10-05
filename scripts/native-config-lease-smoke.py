#!/usr/bin/env python3
"""Public publisher boundary: cancellation and SIGKILL must not leak retention."""
import hashlib
import io
import json
import os
import pathlib
import select
import subprocess
import sys
import tarfile
import tempfile
import time

base = pathlib.Path(tempfile.mkdtemp(prefix="artifactd-config-lease-"))
for name in ("store", "runtime", "publication"):
    (base / name).mkdir(mode=0o700)


def encode(value):
    return json.dumps(value, separators=(",", ":")).encode()


def digest(data):
    return "sha256:" + hashlib.sha256(data).hexdigest()


config = encode({"architecture": "amd64", "os": "linux", "rootfs": {"type": "layers", "diff_ids": []}})
manifest = encode({"schemaVersion": 2, "mediaType": "application/vnd.oci.image.manifest.v1+json",
                   "config": {"mediaType": "application/vnd.oci.image.config.v1+json",
                              "digest": digest(config), "size": len(config)}, "layers": []})
index = encode({"schemaVersion": 2, "mediaType": "application/vnd.oci.image.index.v1+json",
                "manifests": [{"mediaType": "application/vnd.oci.image.manifest.v1+json",
                               "digest": digest(manifest), "size": len(manifest),
                               "platform": {"os": "linux", "architecture": "amd64"}}]})
archive = base / "publication/image.tar"
with tarfile.open(archive, "w") as tar:
    for name, data in (("oci-layout", encode({"imageLayoutVersion": "1.0.0"})), ("index.json", index),
                       ("blobs/sha256/" + digest(config)[7:], config),
                       ("blobs/sha256/" + digest(manifest)[7:], manifest)):
        entry = tarfile.TarInfo(name)
        entry.size, entry.mode = len(data), 0o600
        tar.addfile(entry, io.BytesIO(data))
archive.chmod(0o600)
daemon = pathlib.Path(sys.argv[1]).resolve()
harness = pathlib.Path(sys.argv[2]).resolve()
socket = base / "runtime/artifactd.sock"
receipt = base / "publication/receipt.json"
process = None
child = None


def start():
    global process
    process = subprocess.Popen([str(daemon), "--store", str(base / "store"), "--socket", str(socket)],
                               stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)
    for _ in range(500):
        if process.poll() is not None:
            raise RuntimeError(process.stderr.read().decode())
        try:
            action({"operation": "STATUS"})
            return
        except AssertionError:
            time.sleep(0.01)
    raise RuntimeError("daemon readiness deadline")


def action(value):
    result = subprocess.run([str(daemon.with_name("apollo-artifactctl")), "--socket", str(socket),
                             "--action", json.dumps(value)], capture_output=True, text=True, timeout=45)
    assert result.returncode == 0, result.stderr
    return json.loads(result.stdout)["result"]["Ok"]


def command(mode):
    return [str(harness), str(socket), str(os.geteuid()), str(archive), str(receipt), mode]


def run(mode):
    result = subprocess.run(command(mode), capture_output=True, text=True, timeout=90)
    assert result.returncode == 0, result.stderr
    return result.stdout.strip()


try:
    start()
    normal = json.loads(run("normal"))
    assert normal["config"] == json.loads(config)
    assert normal["receipt"]["config"]["digest"] == digest(config)
    assert run("cancel") == "CANCELLED"
    cancelled = json.loads((base / "publication/config-lease.json.artifactd-operation.json").read_text())
    refused = subprocess.run([str(daemon.with_name("apollo-artifactctl")), "--socket", str(socket),
                              "--action", json.dumps({"operation": "OPEN_BLOB", "digest": digest(config),
                                                     "lease": cancelled["lease"]})],
                             capture_output=True, text=True, timeout=45)
    assert refused.returncode != 0, "cancellation must release its own lease before another read"
    child = subprocess.Popen(command("crash"), stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
    assert select.select([child.stdout], [], [], 45)[0], "lease observation deadline"
    assert child.stdout.readline().strip() == "LEASE_CREATED", "must kill after an actual leased FD opens"
    child.kill()
    child.wait(timeout=10)
    intent = json.loads((base / "publication/receipt.json.artifactd-operation.json").read_text())
    action({"operation": "UNPIN", "id": intent["pin"]})
    action({"operation": "GC", "max_entries": 4096})
    assert action({"operation": "STATUS"})["blobs"] > 0, "lease must cover content after caller death"
    process.kill()
    process.wait(timeout=10)
    start()
    action({"operation": "GC", "max_entries": 4096})
    assert action({"operation": "STATUS"})["blobs"] > 0, "lease must survive daemon restart"
    assert run("reconcile") == "RECONCILED"
    assert run("reconcile") == "RECONCILED"
    action({"operation": "GC", "max_entries": 4096})
    assert action({"operation": "STATUS"})["blobs"] == 0, "durable caller cleanup must release retention"
    print("PASS verified config FD; cancellation cleanup; caller SIGKILL; daemon restart; lease protects graph; repeated recovery releases GC retention")
    print("qualification fixture:", base)
finally:
    for owned in (child, process):
        if owned is not None and owned.poll() is None:
            owned.kill()
            owned.wait(timeout=10)
