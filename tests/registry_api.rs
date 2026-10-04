#![cfg(target_os = "linux")]

#[path = "support/daemon.rs"]
mod daemon;
#[path = "support/registry_fixture.rs"]
mod registry_fixture;

use apollo_artifactd::state::{Operation, State};
use artifactd_protocol::{
    Action, ArtifactDigest, PinId, Platform, PreparedDigest, Request, VERSION, client, client::call,
};
use daemon::Daemon;
use registry_fixture::{archive, pinned, private_credentials};
use std::{
    io::{Seek, SeekFrom},
    os::unix::fs::PermissionsExt,
    path::Path,
};

fn request(id: &str, action: Action) -> Request {
    request_id(id.to_owned().try_into().unwrap(), action)
}

fn request_id(operation_id: artifactd_protocol::OperationId, action: Action) -> Request {
    Request {
        version: VERSION,
        operation_id,
        action,
    }
}

fn mutation(socket: &std::path::Path, action: Action) -> Request {
    request_id(client::allocate(socket).unwrap(), action)
}

fn stop_and_state(root: &Path, daemon: Daemon) -> State {
    drop(daemon);
    let dir = cap_std::fs::Dir::open_ambient_dir(root, cap_std::ambient_authority()).unwrap();
    State::open(&dir).unwrap()
}

#[test]
#[ignore = "requires isolated authenticated HTTPS registry and private credential provider"]
fn registry_api_fd_credentials_journal_and_prepared_facts() {
    let reference = std::env::var("ARTIFACTD_TEST_REGISTRY_REFERENCE").unwrap();
    let credential_path = std::env::var("ARTIFACTD_TEST_REGISTRY_CREDENTIALS").unwrap();

    let source = tempfile::tempdir().unwrap();
    let runtime = tempfile::tempdir().unwrap();
    for dir in [source.path(), runtime.path()] {
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    let (daemon, socket) = Daemon::spawn(source.path(), runtime.path());
    let (archive, expected_root) = archive();
    let imported = call(
        &socket,
        &mutation(
            &socket,
            Action::ImportOciArchive {
                platform: Platform {
                    os: "linux".into(),
                    architecture: "amd64".into(),
                    variant: None,
                },
                pin: None,
            },
        ),
        Some(archive.as_file()),
    )
    .unwrap()
    .0
    .result
    .unwrap();
    let root: ArtifactDigest = serde_json::from_value(imported["artifact_digest"].clone()).unwrap();
    assert_eq!(root, expected_root);

    let credentials = private_credentials(Path::new(&credential_path));
    let push_request = mutation(
        &socket,
        Action::Push {
            digest: root.clone(),
            reference: reference.clone(),
        },
    );
    let push_operation = push_request.operation_id.clone();
    let pushed = call(&socket, &push_request, Some(&credentials))
        .unwrap()
        .0
        .result
        .unwrap();
    assert_eq!(pushed["artifact_digest"], serde_json::json!(root));
    drop(credentials);
    let state = stop_and_state(source.path(), daemon);
    assert!(
        state
            .get::<Operation>("operations", push_operation.as_str())
            .unwrap()
            .is_some()
    );
    drop(state);

    let target = tempfile::tempdir().unwrap();
    let target_runtime = tempfile::tempdir().unwrap();
    for dir in [target.path(), target_runtime.path()] {
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    let (target_daemon, target_socket) = Daemon::spawn(target.path(), target_runtime.path());
    let pinned = pinned(&reference, &root);
    let pin: PinId = "registry-api-pull".to_owned().try_into().unwrap();
    let credentials = private_credentials(Path::new(&credential_path));
    let pulled = call(
        &target_socket,
        &mutation(
            &target_socket,
            Action::Pull {
                reference: pinned.clone(),
                platform: Platform {
                    os: "linux".into(),
                    architecture: "amd64".into(),
                    variant: None,
                },
                pin: Some(pin.clone()),
            },
        ),
        Some(&credentials),
    )
    .unwrap()
    .0
    .result
    .unwrap();
    assert_eq!(pulled["artifact_digest"], serde_json::json!(root));
    assert_eq!(pulled["pin_id"], serde_json::json!(pin));
    let collected = call(
        &target_socket,
        &mutation(&target_socket, Action::Gc { max_entries: 4096 }),
        None,
    )
    .unwrap()
    .0
    .result
    .unwrap();
    assert!(collected["collected"].is_number());
    let verified = call(
        &target_socket,
        &request(
            "verify-after-pull-gc",
            Action::Verify {
                digest: root.clone(),
            },
        ),
        None,
    )
    .unwrap()
    .0
    .result
    .unwrap();
    assert_eq!(verified["artifact_digest"], serde_json::json!(root));

    // Reopen before preparation: the pin must remain a durable GC root.
    drop(target_daemon);
    let target_runtime_restart = tempfile::tempdir().unwrap();
    std::fs::set_permissions(
        target_runtime_restart.path(),
        std::fs::Permissions::from_mode(0o700),
    )
    .unwrap();
    let (target_daemon, target_socket) =
        Daemon::spawn(target.path(), target_runtime_restart.path());
    let _ = call(
        &target_socket,
        &mutation(&target_socket, Action::Gc { max_entries: 4096 }),
        None,
    )
    .unwrap()
    .0
    .result
    .unwrap();
    let verified = call(
        &target_socket,
        &request(
            "verify-after-restart-gc",
            Action::Verify {
                digest: root.clone(),
            },
        ),
        None,
    )
    .unwrap()
    .0
    .result
    .unwrap();
    assert_eq!(verified["artifact_digest"], serde_json::json!(root));

    // A fresh operation must bind the already-local graph to a new pin.
    let cache_pin: PinId = "registry-api-cache".to_owned().try_into().unwrap();
    let cached = call(
        &target_socket,
        &mutation(
            &target_socket,
            Action::Pull {
                reference: pinned,
                platform: Platform {
                    os: "linux".into(),
                    architecture: "amd64".into(),
                    variant: None,
                },
                pin: Some(cache_pin.clone()),
            },
        ),
        Some(&private_credentials(Path::new(&credential_path))),
    )
    .unwrap()
    .0
    .result
    .unwrap();
    assert_eq!(cached["artifact_digest"], serde_json::json!(root));
    assert_eq!(cached["pin_id"], serde_json::json!(cache_pin));
    let _ = call(
        &target_socket,
        &mutation(&target_socket, Action::Unpin { id: pin }),
        None,
    )
    .unwrap()
    .0
    .result
    .unwrap();
    let _ = call(
        &target_socket,
        &mutation(&target_socket, Action::Gc { max_entries: 4096 }),
        None,
    )
    .unwrap()
    .0
    .result
    .unwrap();
    let verified = call(
        &target_socket,
        &request(
            "verify-after-cache-pin-gc",
            Action::Verify {
                digest: root.clone(),
            },
        ),
        None,
    )
    .unwrap()
    .0
    .result
    .unwrap();
    assert_eq!(verified["artifact_digest"], serde_json::json!(root));
    let resolved = call(
        &target_socket,
        &request(
            "resolve",
            Action::Resolve {
                digest: root.clone(),
                platform: Platform {
                    os: "linux".into(),
                    architecture: "amd64".into(),
                    variant: None,
                },
            },
        ),
        None,
    )
    .unwrap()
    .0
    .result
    .unwrap();
    let manifest: ArtifactDigest =
        serde_json::from_value(resolved["manifest_digest"].clone()).unwrap();
    let prepared = call(
        &target_socket,
        &mutation(
            &target_socket,
            Action::Prepare {
                digest: manifest,
                platform: Platform {
                    os: "linux".into(),
                    architecture: "amd64".into(),
                    variant: None,
                },
            },
        ),
        None,
    )
    .unwrap()
    .0
    .result
    .unwrap();
    let _: PreparedDigest = serde_json::from_value(prepared["prepared_digest"].clone()).unwrap();
    drop(credentials);

    let mut bad_credentials =
        serde_json::from_slice::<serde_json::Value>(&std::fs::read(&credential_path).unwrap())
            .unwrap();
    let secret = "artifactd-api-auth-failure-sentinel";
    bad_credentials["password"] = serde_json::json!(secret);
    let mut bad_file = tempfile::NamedTempFile::new().unwrap();
    serde_json::to_writer(bad_file.as_file(), &bad_credentials).unwrap();
    bad_file.as_file_mut().seek(SeekFrom::Start(0)).unwrap();
    std::fs::set_permissions(bad_file.path(), std::fs::Permissions::from_mode(0o600)).unwrap();
    let auth_request = mutation(
        &target_socket,
        Action::Push {
            digest: root.clone(),
            reference: reference.clone(),
        },
    );
    let auth_operation = auth_request.operation_id.clone();
    let auth_failure = call(&target_socket, &auth_request, Some(bad_file.as_file()))
        .unwrap()
        .0
        .result
        .unwrap_err();
    assert!(!auth_failure.contains(secret));

    let invalid_request = mutation(
        &target_socket,
        Action::Pull {
            reference: "https://user:secret@example.invalid/repo:tag".into(),
            platform: Platform {
                os: "linux".into(),
                architecture: "amd64".into(),
                variant: None,
            },
            pin: None,
        },
    );
    let invalid_operation = invalid_request.operation_id.clone();
    let invalid = call(&target_socket, &invalid_request, None)
        .unwrap()
        .0
        .result
        .unwrap_err();
    assert!(!invalid.contains("secret"));
    drop(target_daemon);
    let target_dir =
        cap_std::fs::Dir::open_ambient_dir(target.path(), cap_std::ambient_authority()).unwrap();
    let target_state = State::open(&target_dir).unwrap();
    let reservation = target_state
        .get::<Operation>("operations", invalid_operation.as_str())
        .unwrap()
        .unwrap();
    assert_eq!(reservation.phase, "allocated");
    assert!(reservation.request.is_empty());
    assert!(reservation.result.is_none());
    let failed = target_state
        .get::<Operation>("operations", auth_operation.as_str())
        .unwrap()
        .unwrap();
    assert!(!serde_json::to_string(&failed).unwrap().contains(secret));
    drop(target_state);
}
