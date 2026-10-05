#!/usr/bin/env python3
"""Public API ENOSPC/restart/replay qualification on a private Linux tmpfs.

Requires passwordless sudo for mounting only this newly created fixture.
No production endpoint, database mutation, or fault-injection code is used.
"""
import hashlib
import json
import os
from pathlib import Path
import platform
import subprocess
import sys
import tempfile
import time


assert platform.system() == "Linux" and os.geteuid() != 0
daemon = Path(sys.argv[1]).resolve(strict=True)
ctl = daemon.with_name("apollo-artifactctl")
base = Path(tempfile.mkdtemp(prefix="artifactd-enospc-"))
volume, runtime = base / "volume", base / "runtime"
volume.mkdir(mode=0o700)
runtime.mkdir(mode=0o700)
socket = runtime / "artifactd.sock"
source = base / "input.blob"
size = 8 << 20
hasher = hashlib.sha256()
with source.open("xb") as output:
    for block in range(size // 65536):
        chunk = hashlib.sha256(str(block).encode()).digest() * 2048
        output.write(chunk)
        hasher.update(chunk)
source.chmod(0o600)
digest = "sha256:" + hasher.hexdigest()
process = None
mounted = False
log = (base / "daemon.log").open("ab", buffering=0)


def action(value, operation=None, input_path=None, ok=True):
    command = [str(ctl), "--socket", str(socket), "--action", json.dumps(value)]
    if operation:
        command += ["--operation-id", operation]
    if input_path:
        command += ["--input", str(input_path)]
    result = subprocess.run(command, capture_output=True, text=True, timeout=45)
    assert result.stdout, result.stderr
    envelope = json.loads(result.stdout)
    assert (result.returncode == 0) == ok, envelope
    assert ("Ok" in envelope["result"]) == ok, envelope
    return envelope


def start():
    global process
    process = subprocess.Popen([str(daemon), "--store", str(volume / "store"),
                                "--socket", str(socket)], stdout=log, stderr=log)
    for _ in range(500):
        assert process.poll() is None, (base / "daemon.log").read_text()
        if socket.exists():
            try:
                action({"operation": "STATUS"})
                return
            except AssertionError:
                pass
        time.sleep(0.01)
    raise AssertionError("daemon readiness deadline")


def stop():
    global process
    if process is not None and process.poll() is None:
        process.kill()
        process.wait(timeout=10)
    process = None


try:
    subprocess.run(["sudo", "-n", "mount", "-t", "tmpfs", "-o",
                    f"size=24M,mode=0700,uid={os.geteuid()},gid={os.getegid()}",
                    "tmpfs", str(volume)], check=True, timeout=15)
    mounted = True
    (volume / "store").mkdir(mode=0o700)
    start()
    token = action({"operation": "OPERATION_ALLOCATE"})["result"]["Ok"]["operation_id"]
    # Allocate the journal token while healthy; exhaustion must happen in the
    # real import effect rather than rejecting an unallocated operation.
    filesystem = os.statvfs(volume)
    free = filesystem.f_bavail * filesystem.f_frsize
    reserve = 4 << 20
    assert free > reserve + size
    filler = volume / "owned-filler"
    with filler.open("xb") as output:
        remaining = free - reserve
        block = bytes(65536)
        while remaining:
            count = min(len(block), remaining)
            output.write(block[:count])
            remaining -= count
        output.flush()
        os.fsync(output.fileno())
    request = {"operation": "IMPORT_BLOB", "digest": digest, "size": size}
    failed = action(request, token, source, ok=False)
    assert "No space left on device" in failed["result"]["Err"], failed
    assert action({"operation": "STATUS"})["result"]["Ok"]["blobs"] == 0
    action({"operation": "INSPECT", "digest": digest}, ok=False)
    assert not (volume / "store/blobs" / digest[7:]).exists(), "failed content published"
    stop()
    filler.unlink()
    start()
    action({"operation": "RECONCILE", "max_operations": 4096})
    assert action(request, token, source, ok=False)["result"] == failed["result"], "failed replay changed"
    action({"operation": "INSPECT", "digest": digest}, ok=False)
    action(request, input_path=source)
    verified = action({"operation": "VERIFY", "digest": digest})["result"]["Ok"]
    assert verified["size"] == size and verified["verified"] is True
    doctor = action({"operation": "DOCTOR"})["result"]["Ok"]
    assert doctor["database_integrity_clean"] and doctor["operation_journal"]["interrupted"] == 0
    print(json.dumps({"result": "PASS", "architecture": platform.machine(),
                      "fixture": str(base), "bytes": size, "digest": digest,
                      "daemon_sha256": hashlib.sha256(daemon.read_bytes()).hexdigest(),
                      "ctl_sha256": hashlib.sha256(ctl.read_bytes()).hexdigest(),
                      "failure": failed["result"], "verified": verified, "doctor": doctor}, sort_keys=True))
finally:
    stop()
    log.close()
    if mounted:
        subprocess.run(["sudo", "-n", "umount", str(volume)], check=True, timeout=15)
