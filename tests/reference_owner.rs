#![cfg(target_os = "linux")]

#[path = "support/daemon.rs"]
mod daemon;

use apollo_artifactd::state::State;
use artifactd_protocol::{Action, ArtifactDigest, VERSION, client};
use daemon::Daemon;
use std::{io::Write, os::unix::fs::PermissionsExt, path::Path};

fn mutation(socket: &Path, action: Action) -> artifactd_protocol::Request {
    artifactd_protocol::Request {
        version: VERSION,
        operation_id: client::allocate(socket).unwrap(),
        action,
    }
}

#[test]
fn ownerless_reference_fails_closed_during_gc_without_deleting_blob() {
    let store = tempfile::tempdir().unwrap();
    let runtime = tempfile::tempdir().unwrap();
    for path in [store.path(), runtime.path()] {
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    let (daemon, socket) = Daemon::spawn(store.path(), runtime.path());
    let mut input = tempfile::NamedTempFile::new().unwrap();
    input.write_all(b"legacy-reference").unwrap();
    input.as_file_mut().sync_all().unwrap();
    let import = mutation(
        &socket,
        Action::ImportBlob {
            digest: None,
            size: b"legacy-reference".len() as u64,
        },
    );
    let import_input = std::fs::File::open(input.path()).unwrap();
    let digest: ArtifactDigest = serde_json::from_value(
        client::call(&socket, &import, Some(&import_input))
            .unwrap()
            .0
            .result
            .unwrap()["artifact_digest"]
            .clone(),
    )
    .unwrap();
    let pin: artifactd_protocol::PinId = "zz-legacy-pin".to_owned().try_into().unwrap();
    let pin_request = mutation(
        &socket,
        Action::Pin {
            id: pin.clone(),
            digest: digest.clone(),
        },
    );
    assert!(
        client::call(&socket, &pin_request, None)
            .unwrap()
            .0
            .result
            .is_ok()
    );
    for index in 0..65 {
        let request = mutation(
            &socket,
            Action::Pin {
                id: format!("owned-{index:03}").try_into().unwrap(),
                digest: digest.clone(),
            },
        );
        assert!(
            client::call(&socket, &request, None)
                .unwrap()
                .0
                .result
                .is_ok()
        );
    }
    drop(daemon);

    let dir =
        cap_std::fs::Dir::open_ambient_dir(store.path(), cap_std::ambient_authority()).unwrap();
    let state = State::open(&dir).unwrap();
    let mut raw: serde_json::Value = state.get("pins", pin.as_str()).unwrap().unwrap();
    raw.as_object_mut().unwrap().remove("owner");
    state.put("pins", pin.as_str(), &raw).unwrap();
    drop(state);

    let restart_runtime = tempfile::tempdir().unwrap();
    std::fs::set_permissions(
        restart_runtime.path(),
        std::fs::Permissions::from_mode(0o700),
    )
    .unwrap();
    let (daemon, restart_socket) = Daemon::spawn(store.path(), restart_runtime.path());
    let gc = (0..4)
        .map(|_| {
            client::call(
                &restart_socket,
                &mutation(&restart_socket, Action::Gc { max_entries: 4096 }),
                None,
            )
            .unwrap()
            .0
            .result
        })
        .find_map(Result::err)
        .expect("bounded GC must reach the late legacy root");
    assert!(gc.contains("legacy reference ownership requires explicit migration"));
    let verify = client::call(
        &restart_socket,
        &artifactd_protocol::Request {
            version: VERSION,
            operation_id: "legacy-verify".to_owned().try_into().unwrap(),
            action: Action::Verify { digest },
        },
        None,
    )
    .unwrap()
    .0
    .result;
    assert!(verify.is_ok());
    drop(daemon);
}
