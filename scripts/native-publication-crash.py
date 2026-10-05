#!/usr/bin/env python3
"""Kill the actual daemon after kernel CAS publication, before completion.

strace delays the return from linkat; it never changes the syscall result.
All state is produced through the public API in a newly owned fixture.
"""
import hashlib
import io
import json
import os
from pathlib import Path
import platform
import subprocess
import socket as sockets
import sys
import tarfile
import tempfile
import time
import uuid
import array

assert platform.system() == "Linux" and os.geteuid() != 0
daemon = Path(sys.argv[1]).resolve(strict=True)
ctl = daemon.with_name("apollo-artifactctl")
mode = sys.argv[2] if len(sys.argv) > 2 else "blob"
assert mode in ("blob", "prepared", "gc")
prepared_mode = mode != "blob"
base = Path(tempfile.mkdtemp(prefix="artifactd-publish-crash-"))
store, runtime = base / "store", base / "runtime"
store.mkdir(mode=0o700)
runtime.mkdir(mode=0o700)
socket = runtime / "artifactd.sock"
source = base / "input.blob"
content = hashlib.sha256(b"publication-crash-fixture").digest() * 32768
source.write_bytes(content)
source.chmod(0o600)
digest = "sha256:" + hashlib.sha256(content).hexdigest()
leaf = store / "blobs" / digest[7:]
image_platform = {"os": "linux", "architecture": "amd64"}
log = (base / "daemon.log").open("ab", buffering=0)
process = importer = None
traced_pid = None


def command(action, token=None, input_path=None):
    args = [str(ctl), "--socket", str(socket), "--action", json.dumps(action)]
    if token:
        args += ["--operation-id", token]
    if input_path:
        args += ["--input", str(input_path)]
    return args


def call(action, token=None, input_path=None, ok=True):
    result = subprocess.run(command(action, token, input_path), capture_output=True,
                            text=True, timeout=30)
    assert result.stdout, result.stderr
    response = json.loads(result.stdout)
    assert (result.returncode == 0) == ok, response
    assert ("Ok" in response["result"]) == ok, response
    return response["result"]


def start(traced=False):
    global process, traced_pid
    args = [str(daemon), "--store", str(store), "--socket", str(socket)]
    if traced:
        syscall = {"blob": "linkat", "prepared": "renameat", "gc": "unlinkat"}[mode]
        args = ["strace", "-o", str(base / "publication.strace"),
                "-e", f"trace={syscall}", "-e", f"inject={syscall}:delay_exit=10s:when=1"] + args
    process = subprocess.Popen(args, stdout=log, stderr=log)
    for _ in range(500):
        assert process.poll() is None, (base / "daemon.log").read_text()
        if traced and traced_pid is None:
            children = Path(f"/proc/{process.pid}/task/{process.pid}/children").read_text().split()
            if children:
                assert len(children) == 1, children
                traced_pid = int(children[0])
                assert Path(f"/proc/{traced_pid}/exe").resolve() == daemon
        if socket.exists():
            try:
                call({"operation": "STATUS"})
                return
            except AssertionError:
                pass
        time.sleep(0.01)
    raise AssertionError("daemon readiness deadline")


def stop():
    global process, traced_pid
    if traced_pid is not None and Path(f"/proc/{traced_pid}").exists():
        # The child identity was captured from this fixture's strace process.
        assert Path(f"/proc/{traced_pid}/exe").resolve() == daemon
        os.kill(traced_pid, 9)
    if process is not None:
        if process.poll() is None:
            if traced_pid is None:
                process.kill()
            process.wait(timeout=15)
    process = None
    traced_pid = None


def image():
    """Independent standard OCI fixtures; never execute their configuration."""
    def identity(data):
        return "sha256:" + hashlib.sha256(data).hexdigest()

    def encode(value):
        return json.dumps(value, separators=(",", ":")).encode()

    def descriptor(data, media_type):
        return {"digest": identity(data), "size": len(data), "mediaType": media_type}

    layer = io.BytesIO()
    with tarfile.open(fileobj=layer, mode="w", format=tarfile.USTAR_FORMAT) as archive:
        entry = tarfile.TarInfo("verified-file")
        entry.size, entry.mode = len(content), 0o644
        archive.addfile(entry, io.BytesIO(content))
    layer = layer.getvalue()
    config = encode({**image_platform, "rootfs": {"type": "layers", "diff_ids": [identity(layer)]}})
    manifest = encode({"schemaVersion": 2, "mediaType": "application/vnd.oci.image.manifest.v1+json",
                       "config": descriptor(config, "application/vnd.oci.image.config.v1+json"),
                       "layers": [descriptor(layer, "application/vnd.oci.image.layer.v1.tar")]})
    for data in (layer, config, manifest):
        source.write_bytes(data)
        call({"operation": "IMPORT_BLOB", "digest": identity(data), "size": len(data)}, input_path=source)
    result = identity(manifest)
    call({"operation": "IMPORT_OCI", "digest": result, "platform": image_platform})
    return result


def verify_prepared_fd(prepared_id, manifest_digest):
    lease = call({"operation": "LEASE_CREATE", "digest": manifest_digest})["Ok"]["lease_id"]
    # Exercise the public descriptor boundary; the CLI intentionally closes its FD.
    with sockets.socket(sockets.AF_UNIX, sockets.SOCK_SEQPACKET) as connection:
        connection.settimeout(30)
        connection.connect(str(socket))
        connection.send(json.dumps({"version": 3, "operation_id": str(uuid.uuid4()),
                                   "action": {"operation": "OPEN_PREPARED", "id": prepared_id,
                                              "lease": lease}}).encode())
        response, ancillary, flags, _ = connection.recvmsg(65536, sockets.CMSG_SPACE(4))
    assert not flags & (sockets.MSG_TRUNC | sockets.MSG_CTRUNC)
    assert "Ok" in json.loads(response)["result"], response
    descriptors = array.array("i")
    for level, kind, data in ancillary:
        assert level == sockets.SOL_SOCKET and kind == sockets.SCM_RIGHTS
        descriptors.frombytes(data)
    assert len(descriptors) == 1
    root_fd = descriptors[0]
    try:
        assert os.fstat(root_fd).st_mode & 0o222 == 0
        file_fd = os.open("verified-file", os.O_RDONLY | os.O_NOFOLLOW, dir_fd=root_fd)
        with os.fdopen(file_fd, "rb") as file:
            assert file.read() == content
    finally:
        os.close(root_fd)
        call({"operation": "LEASE_RELEASE", "id": lease})


try:
    if prepared_mode:
        start()
        digest = image()
        if mode == "gc":
            receipt = call({"operation": "PREPARE", "digest": digest, "platform": image_platform})["Ok"]
            leaf = store / "prepared" / receipt["prepared_artifact_id"]
            assert (leaf / "verified-file").read_bytes() == content
        stop()
        if mode == "gc":
            # Remove only this stopped fixture's stale socket so the first
            # traced unlink is the intended GC effect, not listener setup.
            socket.unlink()
    start(traced=True)
    token = call({"operation": "OPERATION_ALLOCATE"})["Ok"]["operation_id"]
    request = ({"operation": "PREPARE", "digest": digest, "platform": image_platform}
               if prepared_mode else {"operation": "IMPORT_BLOB", "digest": digest, "size": len(content)})
    if mode == "gc":
        request = {"operation": "GC", "max_entries": 4096}
    importer = subprocess.Popen(command(request, token, None if prepared_mode else source), stdout=subprocess.PIPE,
                                stderr=subprocess.PIPE, text=True)
    deadline = time.monotonic() + 15
    while True:
        if mode == "gc":
            if not (leaf / "verified-file").exists():
                assert leaf.is_dir(), "no partial-tree deletion window"
                break
        elif prepared_mode:
            published = [entry for entry in (store / "prepared").iterdir() if not entry.name.endswith(".staging")]
            if published:
                assert len(published) == 1
                leaf = published[0]
                break
        elif leaf.exists():
            break
        assert importer.poll() is None, importer.communicate()
        assert time.monotonic() < deadline, "publication syscall not reached"
        time.sleep(0.005)
    metadata = leaf.stat()
    if mode == "gc":
        assert not (leaf / "verified-file").exists()
    elif prepared_mode:
        assert metadata.st_mode & 0o222 == 0
        assert (leaf / "verified-file").read_bytes() == content
    else:
        assert metadata.st_mode & 0o222 == 0
        assert metadata.st_nlink == 2
        assert hashlib.sha256(leaf.read_bytes()).hexdigest() == digest[7:]
    assert importer.poll() is None, "operation already completed"
    stop()
    stdout, stderr = importer.communicate(timeout=15)
    assert importer.returncode != 0, (stdout, stderr)
    start()
    call({"operation": "RECONCILE", "max_operations": 4096})
    replay = call(request, token, None if prepared_mode else source, ok=False)
    assert "outcome uncertain" in replay["Err"], replay
    if mode == "gc":
        assert not leaf.exists(), "recovery left partial prepared tree"
        for _ in range(8):
            call(request)
            verified = call({"operation": "STATUS"})["Ok"]
            if verified["blobs"] == 0:
                break
        assert verified["blobs"] == 0, verified
        assert not list((store / "prepared").iterdir())
    elif prepared_mode:
        verified = call(request)["Ok"]
        assert verified["prepared_artifact_id"] == leaf.name
        assert verified["manifest_digest"] == digest
        verify_prepared_fd(leaf.name, digest)
        assert not any(entry.name.endswith(".staging") for entry in (store / "prepared").iterdir())
    else:
        verified = call({"operation": "VERIFY", "digest": digest})["Ok"]
        assert verified["verified"] and verified["size"] == len(content)
        call(request, input_path=source)
        assert leaf.stat().st_nlink == 1
    assert not list((store / "temp").iterdir())
    doctor = call({"operation": "DOCTOR"})["Ok"]
    assert doctor["database_integrity_clean"]
    print(json.dumps({"result": "PASS", "architecture": platform.machine(), "mode": mode,
                      "fixture": str(base), "digest": digest, "bytes": len(content),
                      "daemon_sha256": hashlib.sha256(daemon.read_bytes()).hexdigest(),
                      "ctl_sha256": hashlib.sha256(ctl.read_bytes()).hexdigest(),
                      "replay": replay, "verified": verified, "doctor": doctor}, sort_keys=True))
finally:
    stop()
    if importer is not None and importer.poll() is None:
        importer.kill()
        importer.wait(timeout=10)
    log.close()
