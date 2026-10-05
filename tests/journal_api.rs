#![cfg(target_os = "linux")]

#[path = "support/daemon.rs"]
mod daemon;

use apollo_artifactd::state::{Operation, State};
use artifactd_protocol::{Action, ArtifactDigest, OperationId, Platform, Request, VERSION, client};
use daemon::Daemon;
use std::{
    os::unix::fs::PermissionsExt,
    process::{Command, Stdio},
    time::Duration,
};

fn request(operation_id: OperationId, action: Action) -> Request {
    Request {
        version: VERSION,
        operation_id,
        action,
    }
}

fn mutation(socket: &std::path::Path, action: Action) -> Request {
    request(client::allocate(socket).unwrap(), action)
}

fn root() -> (tempfile::TempDir, tempfile::TempDir) {
    let store = tempfile::tempdir().unwrap();
    let runtime = tempfile::tempdir().unwrap();
    for path in [store.path(), runtime.path()] {
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    (store, runtime)
}

fn wait_for_refusal(mut child: std::process::Child) {
    for _ in 0..100 {
        if let Some(status) = child.try_wait().unwrap() {
            assert!(!status.success(), "corrupt state unexpectedly accepted");
            return;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    let _ = child.kill();
    let _ = child.wait();
    panic!("daemon accepted corrupt state");
}

#[test]
fn allocated_mutations_replay_conflict_and_restart_with_bounded_journal() {
    let (store, runtime) = root();
    let (daemon, socket) = Daemon::spawn(store.path(), runtime.path());

    let absent: ArtifactDigest = format!("sha256:{}", "0".repeat(64)).parse().unwrap();
    let first = mutation(
        &socket,
        Action::Pin {
            id: "missing-first".to_owned().try_into().unwrap(),
            digest: absent.clone(),
        },
    );
    let first_result = client::call(&socket, &first, None).unwrap().0.result;
    assert!(first_result.is_err());
    assert_eq!(
        first_result,
        client::call(&socket, &first, None).unwrap().0.result,
        "failed mutation must replay its terminal result"
    );
    let conflict = request(
        first.operation_id.clone(),
        Action::Pin {
            id: "missing-conflict".to_owned().try_into().unwrap(),
            digest: absent,
        },
    );
    assert!(
        client::call(&socket, &conflict, None)
            .unwrap()
            .0
            .result
            .is_err()
    );

    let ancient: OperationId = "00000000000000000000000000000000_00000000000000000000"
        .to_owned()
        .try_into()
        .unwrap();
    let ancient_request = request(
        ancient,
        Action::Unpin {
            id: "ancient".to_owned().try_into().unwrap(),
        },
    );
    assert!(
        client::call(&socket, &ancient_request, None)
            .unwrap()
            .0
            .result
            .is_err()
    );

    // Reproduce a lost completion commit at the persisted state boundary.
    let interrupted = mutation(
        &socket,
        Action::Unpin {
            id: "interrupted".to_owned().try_into().unwrap(),
        },
    );
    let interrupted_id = interrupted.operation_id.clone();
    drop(daemon);
    let dir =
        cap_std::fs::Dir::open_ambient_dir(store.path(), cap_std::ambient_authority()).unwrap();
    let state = State::open(&dir).unwrap();
    let mut record: Operation = state
        .get("operations", interrupted_id.as_str())
        .unwrap()
        .unwrap();
    record.phase = "intent".into();
    record.request = serde_json::to_string(&interrupted.action).unwrap();
    record.result = None;
    state
        .put("operations", interrupted_id.as_str(), &record)
        .unwrap();
    drop(state);

    let recovery_runtime = tempfile::tempdir().unwrap();
    std::fs::set_permissions(
        recovery_runtime.path(),
        std::fs::Permissions::from_mode(0o700),
    )
    .unwrap();
    let (daemon, socket) = Daemon::spawn(store.path(), recovery_runtime.path());
    let uncertain = client::call(&socket, &interrupted, None)
        .unwrap()
        .0
        .result
        .unwrap_err();
    assert!(uncertain.contains("outcome uncertain"), "{uncertain}");

    // Unpin is idempotent, so these are successful public mutations. This is
    // intentionally above the 4096 journal cap and retires the first failure.
    for i in 0..4200 {
        let operation = mutation(
            &socket,
            Action::Unpin {
                id: format!("churn-{i}").try_into().unwrap(),
            },
        );
        assert!(
            client::call(&socket, &operation, None)
                .unwrap()
                .0
                .result
                .is_ok()
        );
    }

    let expired = client::call(&socket, &first, None)
        .unwrap()
        .0
        .result
        .unwrap_err();
    assert!(
        expired.to_ascii_lowercase().contains("expired"),
        "{expired}"
    );
    let interrupted_expired = client::call(&socket, &interrupted, None)
        .unwrap()
        .0
        .result
        .unwrap_err();
    assert!(
        interrupted_expired.to_ascii_lowercase().contains("expired"),
        "{interrupted_expired}"
    );
    let post_restart = mutation(
        &socket,
        Action::Unpin {
            id: "restart-mutation".to_owned().try_into().unwrap(),
        },
    );
    assert!(
        client::call(&socket, &post_restart, None)
            .unwrap()
            .0
            .result
            .is_ok()
    );

    drop(daemon);
    let dir =
        cap_std::fs::Dir::open_ambient_dir(store.path(), cap_std::ambient_authority()).unwrap();
    let state = State::open(&dir).unwrap();
    assert!(state.count("operations").unwrap() <= 4096);
    assert!(state.count("registry").unwrap() <= 4096);
    drop(state);

    // A registry operation must have a matching companion journal record.
    // Construct an intent without that companion and require startup refusal.
    let corrupt_runtime = tempfile::tempdir().unwrap();
    std::fs::set_permissions(
        corrupt_runtime.path(),
        std::fs::Permissions::from_mode(0o700),
    )
    .unwrap();
    let (corrupt_daemon, corrupt_socket) = Daemon::spawn(store.path(), corrupt_runtime.path());
    let corrupt = mutation(
        &corrupt_socket,
        Action::Pull {
            reference: "https://registry.invalid/repo:tag".into(),
            platform: Platform {
                os: "linux".into(),
                architecture: "amd64".into(),
                variant: None,
            },
            pin: None,
        },
    );
    let corrupt_id = corrupt.operation_id.clone();
    drop(corrupt_daemon);
    let dir =
        cap_std::fs::Dir::open_ambient_dir(store.path(), cap_std::ambient_authority()).unwrap();
    let state = State::open(&dir).unwrap();
    let mut record: Operation = state
        .get("operations", corrupt_id.as_str())
        .unwrap()
        .unwrap();
    record.phase = "intent".into();
    record.request = serde_json::to_string(&corrupt.action).unwrap();
    record.result = None;
    state
        .put("operations", corrupt_id.as_str(), &record)
        .unwrap();
    drop(state);
    let refused_runtime = tempfile::tempdir().unwrap();
    let refused_socket = refused_runtime.path().join("artifactd.sock");
    let child = Command::new(env!("CARGO_BIN_EXE_apollo-artifactd"))
        .env_clear()
        .args([
            "--store",
            store.path().to_str().unwrap(),
            "--socket",
            refused_socket.to_str().unwrap(),
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    wait_for_refusal(child);
}
