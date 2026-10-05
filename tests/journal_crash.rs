#![cfg(target_os = "linux")]

#[path = "support/daemon.rs"]
mod daemon;

use apollo_artifactd::state::{Operation, State};
use artifactd_protocol::{Action, OperationId, Request, VERSION, client};
use daemon::Daemon;
use std::{
    fs::{self, File},
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    thread,
    time::{Duration, Instant},
};

const SIZE: u64 = 256 * 1024 * 1024;

fn request(operation_id: OperationId, action: Action) -> Request {
    Request {
        version: VERSION,
        operation_id,
        action,
    }
}

fn root() -> (tempfile::TempDir, tempfile::TempDir) {
    let store = tempfile::tempdir().unwrap();
    let runtime = tempfile::tempdir().unwrap();
    for path in [store.path(), runtime.path()] {
        fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
    }
    (store, runtime)
}

fn partial_temp(store: &Path) -> Option<(PathBuf, u64)> {
    fs::read_dir(store.join("temp"))
        .ok()?
        .filter_map(Result::ok)
        .find_map(|entry| {
            let path = entry.path();
            (path.extension().and_then(|ext| ext.to_str()) == Some("part"))
                .then(|| Some((path.clone(), entry.metadata().ok()?.len())))?
        })
}

fn state(store: &Path) -> State {
    let dir = cap_std::fs::Dir::open_ambient_dir(store, cap_std::ambient_authority()).unwrap();
    State::open(&dir).unwrap()
}

#[test]
#[ignore = "native SIGKILL recovery qualification"]
fn sigkill_import_preserves_intent_and_recovers_without_reexecution() {
    for attempt in 0..2 {
        let (store, runtime) = root();
        let input = tempfile::NamedTempFile::new().unwrap();
        input.as_file().set_len(SIZE).unwrap();
        input.as_file().sync_all().unwrap();
        let (mut daemon, socket) = Daemon::spawn(store.path(), runtime.path());
        let operation_id = client::allocate(&socket).unwrap();
        let operation = request(
            operation_id.clone(),
            Action::ImportBlob {
                digest: None,
                size: SIZE,
            },
        );
        let file = File::open(input.path()).unwrap();
        let request_socket = socket.clone();
        let handle = thread::spawn(move || client::call(&request_socket, &operation, Some(&file)));
        let deadline = Instant::now() + Duration::from_secs(30);
        let mut partial = None;
        while Instant::now() < deadline && !handle.is_finished() {
            if let Some(found) = partial_temp(store.path())
                && found.1 > 0
                && found.1 < SIZE
            {
                partial = Some(found);
                break;
            }
            thread::sleep(Duration::from_millis(10));
        }
        let Some((temp_path, temp_size)) = partial else {
            let completed = handle.join().unwrap().unwrap().0.result;
            drop(daemon);
            if completed.is_ok() && attempt == 0 {
                continue;
            }
            panic!("import completed before observable partial temp file: {completed:?}");
        };
        assert!(temp_size < SIZE);
        daemon.0.kill().unwrap();
        daemon.0.wait().unwrap();
        let _ = handle.join();
        assert!(temp_path.exists());

        let persisted: Operation = state(store.path())
            .get("operations", operation_id.as_str())
            .unwrap()
            .unwrap();
        assert_eq!(
            persisted.phase, "intent",
            "SIGKILL must leave durable intent"
        );
        assert!(persisted.result.is_none());

        let restart_runtime = tempfile::tempdir().unwrap();
        fs::set_permissions(restart_runtime.path(), fs::Permissions::from_mode(0o700)).unwrap();
        let (daemon, restart_socket) = Daemon::spawn(store.path(), restart_runtime.path());
        let replay_input = File::open(input.path()).unwrap();
        let uncertain = client::call(
            &restart_socket,
            &request(
                operation_id.clone(),
                Action::ImportBlob {
                    digest: None,
                    size: SIZE,
                },
            ),
            Some(&replay_input),
        )
        .unwrap()
        .0
        .result
        .unwrap_err();
        assert!(uncertain.contains("outcome uncertain"), "{uncertain}");
        assert!(
            fs::read_dir(store.path().join("temp"))
                .unwrap()
                .next()
                .is_none()
        );
        let post_restart = request(
            client::allocate(&restart_socket).unwrap(),
            Action::Unpin {
                id: "after-crash".to_owned().try_into().unwrap(),
            },
        );
        assert!(
            client::call(&restart_socket, &post_restart, None)
                .unwrap()
                .0
                .result
                .is_ok()
        );
        drop(daemon);
        let recovered = state(store.path());
        assert_eq!(recovered.count("imports").unwrap(), 0);
        assert_eq!(recovered.count("blobs").unwrap(), 0);
        let terminal: Operation = recovered
            .get("operations", operation_id.as_str())
            .unwrap()
            .unwrap();
        assert_eq!(terminal.phase, "failed");
        assert!(terminal.result.unwrap().contains("outcome uncertain"));
        return;
    }
    unreachable!();
}
