#![cfg(target_os = "linux")]

use apollo_artifactd::state::{Operation, State};
use artifactd_protocol::{Action, Request, VERSION, client};
use cap_std::fs::Dir;
use std::{
    fs::{self, OpenOptions},
    io::{Seek, SeekFrom, Write},
    os::unix::fs::PermissionsExt,
    path::Path,
    thread,
    time::{Duration, Instant},
};

#[path = "support/daemon.rs"]
mod daemon;
use daemon::Daemon;

fn private_root() -> tempfile::TempDir {
    let root = tempfile::tempdir().unwrap();
    fs::set_permissions(root.path(), fs::Permissions::from_mode(0o700)).unwrap();
    root
}

fn marker_offsets(bytes: &[u8], marker: &[u8]) -> Vec<usize> {
    bytes
        .windows(marker.len())
        .enumerate()
        .filter_map(|(offset, window)| (window == marker).then_some(offset))
        .collect()
}

fn seed_marker(root: &Path) -> String {
    let dir = Dir::open_ambient_dir(root, cap_std::ambient_authority()).unwrap();
    let marker = format!(
        "doctor-integrity-marker-{}-{}",
        uuid::Uuid::new_v4().simple(),
        "x".repeat(24 * 1024)
    );
    let state = State::open(&dir).unwrap();
    state.put("metadata", "doctor_marker", &marker).unwrap();
    assert_eq!(
        state.get::<String>("metadata", "doctor_marker").unwrap(),
        Some(marker.clone())
    );
    drop(state);
    marker
}

fn flip_marker(root: &Path, marker: &str) {
    let state_path = root.join("state.redb");
    let mut bytes = fs::read(&state_path).unwrap();
    let offsets = marker_offsets(&bytes, marker.as_bytes());
    assert_eq!(offsets.len(), 1, "marker must be uniquely committed");
    let offset = offsets[0] + marker.len() / 2;
    bytes[offset] ^= 0x01;
    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(&state_path)
        .unwrap();
    file.seek(SeekFrom::Start(offset as u64)).unwrap();
    file.write_all(&[bytes[offset]]).unwrap();
    file.sync_all().unwrap();
    drop(file);
    assert_eq!(fs::metadata(&state_path).unwrap().len(), bytes.len() as u64);
}

fn corrupt_committed_page(keep_open: bool) {
    let root = private_root();
    let dir = Dir::open_ambient_dir(root.path(), cap_std::ambient_authority()).unwrap();
    let marker = seed_marker(root.path());
    let mut owner = Some(State::open(&dir).unwrap());

    if !keep_open {
        drop(owner.take());
    }
    flip_marker(root.path(), &marker);

    let opened = match owner {
        Some(state) => Ok(state),
        None => State::open(&dir),
    };
    match opened {
        Err(_) => {}
        Ok(mut state) => {
            let doctor = state.check_integrity();
            assert!(
                matches!(doctor, Err(_) | Ok(false)),
                "corrupted state must not be reported clean: {doctor:?}"
            );
            assert!(
                state
                    .put("metadata", "after_corruption", &"must-fail")
                    .is_err(),
                "mutations must remain blocked after doctor detects corruption"
            );
        }
    }
}

#[test]
fn daemon_doctor_corruption_quarantines_and_exits() {
    let root = private_root();
    let marker = seed_marker(root.path());
    let runtime = private_root();
    let (mut daemon, socket) = Daemon::spawn(root.path(), runtime.path());
    let operation_id = client::allocate(&socket).unwrap();
    flip_marker(root.path(), &marker);

    let request = Request {
        version: VERSION,
        operation_id: operation_id.clone(),
        action: Action::Doctor,
    };
    let worker_socket = socket.clone();
    let request_result = thread::spawn(move || client::call(&worker_socket, &request, None))
        .join()
        .unwrap();
    let uncertain = request_result.unwrap().0.result.unwrap_err();
    assert!(uncertain.contains("database quarantined"), "{uncertain}");
    assert!(uncertain.contains("outcome uncertain"), "{uncertain}");

    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        if daemon.0.try_wait().unwrap().is_some() {
            break;
        }
        thread::sleep(Duration::from_millis(10));
    }
    assert!(
        daemon.0.try_wait().unwrap().is_some(),
        "daemon must terminate after quarantining corrupted state"
    );
    let follow_up = Request {
        version: VERSION,
        operation_id: "follow-up".to_owned().try_into().unwrap(),
        action: Action::Status,
    };
    assert!(
        client::call(&socket, &follow_up, None).is_err(),
        "a quarantined daemon must not accept follow-up requests"
    );

    if let Ok(state) =
        State::open(&Dir::open_ambient_dir(root.path(), cap_std::ambient_authority()).unwrap())
    {
        let operation = state
            .get::<Operation>("operations", operation_id.as_str())
            .unwrap()
            .unwrap();
        assert_eq!(operation.phase, "intent");
        assert!(matches!(
            serde_json::from_str::<Action>(&operation.request).unwrap(),
            Action::Doctor
        ));
        assert!(operation.result.is_none());
    }
}

#[test]
fn doctor_fails_closed_after_committed_state_page_corruption() {
    // Reopen rejection and a live-owner check protect distinct lifecycle risks.
    for keep_open in [false, true] {
        corrupt_committed_page(keep_open);
    }
}
