#![cfg(target_os = "linux")]

#[path = "support/daemon.rs"]
mod daemon;

use apollo_artifactd::state::{Operation, PeerIdentity, State};
use artifactd_protocol::{Action, OperationId, Request, VERSION, client};
use daemon::Daemon;
use std::{
    io::Write,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

fn request(operation_id: OperationId, action: Action) -> Request {
    Request {
        version: VERSION,
        operation_id,
        action,
    }
}

fn mutation(socket: &Path, action: Action) -> Request {
    request(client::allocate(socket).unwrap(), action)
}

fn roots() -> (tempfile::TempDir, tempfile::TempDir) {
    let store = tempfile::tempdir().unwrap();
    let runtime = tempfile::tempdir().unwrap();
    for path in [store.path(), runtime.path()] {
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    (store, runtime)
}

fn restart(store: &Path) -> (Daemon, tempfile::TempDir, PathBuf) {
    let runtime = tempfile::tempdir().unwrap();
    std::fs::set_permissions(runtime.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let (daemon, socket) = Daemon::spawn(store, runtime.path());
    (daemon, runtime, socket)
}

fn assert_refused(store: &Path, runtime: &Path) {
    let socket = runtime.join("artifactd.sock");
    let mut child = Command::new(env!("CARGO_BIN_EXE_apollo-artifactd"))
        .env_clear()
        .args([
            "--store",
            store.to_str().unwrap(),
            "--socket",
            socket.to_str().unwrap(),
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    for _ in 0..100 {
        if let Some(status) = child.try_wait().unwrap() {
            assert!(!status.success(), "ownerless journal unexpectedly accepted");
            let output = child.wait_with_output().unwrap();
            assert!(
                String::from_utf8_lossy(&output.stderr)
                    .contains("legacy operation ownership requires explicit migration")
            );
            return;
        }
        let probe = request(
            "ownerless-probe".to_owned().try_into().unwrap(),
            Action::Capabilities,
        );
        if client::call(&socket, &probe, None).is_ok_and(|(response, _)| response.result.is_ok()) {
            let _ = child.kill();
            let _ = child.wait();
            panic!("daemon served ownerless journal");
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    let _ = child.kill();
    let _ = child.wait();
    panic!("daemon accepted ownerless journal");
}

#[test]
fn persisted_operation_owner_mismatch_blocks_effect_and_replay() {
    let (store, runtime) = roots();
    let (daemon, socket) = Daemon::spawn(store.path(), runtime.path());
    let mut input = tempfile::NamedTempFile::new().unwrap();
    input.write_all(b"owner-boundary").unwrap();
    input.as_file_mut().sync_all().unwrap();
    let input_read = std::fs::File::open(input.path()).unwrap();
    let import = mutation(
        &socket,
        Action::ImportBlob {
            digest: None,
            size: input_read.metadata().unwrap().len(),
        },
    );
    let terminal = mutation(
        &socket,
        Action::Unpin {
            id: "owner-terminal".to_owned().try_into().unwrap(),
        },
    );
    assert!(
        client::call(&socket, &terminal, None)
            .unwrap()
            .0
            .result
            .is_ok()
    );
    drop(daemon);

    let dir =
        cap_std::fs::Dir::open_ambient_dir(store.path(), cap_std::ambient_authority()).unwrap();
    let state = State::open(&dir).unwrap();
    let current = PeerIdentity {
        uid: rustix::process::geteuid().as_raw(),
        gid: rustix::process::getegid().as_raw(),
    };
    let foreign = PeerIdentity {
        uid: current.uid.saturating_add(1),
        gid: current.gid,
    };
    let mut expected = Vec::new();
    for id in [&import.operation_id, &terminal.operation_id] {
        let mut operation: Operation = state.get("operations", id.as_str()).unwrap().unwrap();
        assert_eq!(operation.owner, Some(current.clone()));
        operation.owner = Some(foreign.clone());
        state.put("operations", id.as_str(), &operation).unwrap();
        expected.push((id.clone(), operation));
    }
    drop(state);

    let (daemon, _runtime_restart, socket) = restart(store.path());
    let error = client::call(&socket, &import, Some(&input_read))
        .unwrap()
        .0
        .result
        .unwrap_err();
    assert!(error.contains("operation owner mismatch"), "{error}");
    let replay_error = client::call(&socket, &terminal, None)
        .unwrap()
        .0
        .result
        .unwrap_err();
    assert!(
        replay_error.contains("operation owner mismatch"),
        "{replay_error}"
    );
    drop(daemon);

    let dir =
        cap_std::fs::Dir::open_ambient_dir(store.path(), cap_std::ambient_authority()).unwrap();
    let state = State::open(&dir).unwrap();
    assert_eq!(state.count("imports").unwrap(), 0);
    assert_eq!(state.count("blobs").unwrap(), 0);
    for (id, expected) in expected {
        let operation: Operation = state.get("operations", id.as_str()).unwrap().unwrap();
        assert_eq!(operation, expected);
    }
}

#[test]
fn ownerless_persisted_operation_fails_closed_and_is_preserved() {
    let (store, runtime) = roots();
    let (daemon, socket) = Daemon::spawn(store.path(), runtime.path());
    let operation = mutation(
        &socket,
        Action::Unpin {
            id: "ownerless".to_owned().try_into().unwrap(),
        },
    );
    drop(daemon);
    let dir =
        cap_std::fs::Dir::open_ambient_dir(store.path(), cap_std::ambient_authority()).unwrap();
    let state = State::open(&dir).unwrap();
    let mut raw: serde_json::Value = state
        .get("operations", operation.operation_id.as_str())
        .unwrap()
        .unwrap();
    raw.as_object_mut().unwrap().remove("owner");
    state
        .put("operations", operation.operation_id.as_str(), &raw)
        .unwrap();
    drop(state);
    assert_refused(store.path(), runtime.path());
    let state = State::open(&dir).unwrap();
    let preserved: Operation = state
        .get("operations", operation.operation_id.as_str())
        .unwrap()
        .unwrap();
    assert!(preserved.owner.is_none());
}
