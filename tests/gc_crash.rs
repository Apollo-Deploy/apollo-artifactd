#![cfg(target_os = "linux")]

#[path = "support/daemon.rs"]
mod daemon;

use apollo_artifactd::{
    Limits, Store,
    state::{Operation, Prepared, State},
};
use artifactd_protocol::{Action, ArtifactDigest, OperationId, Platform, Request, VERSION, client};
use daemon::Daemon;
use sha2::{Digest, Sha256};
use std::{
    fs::{self, read_dir},
    io::{Cursor, Read},
    os::unix::fs::PermissionsExt,
    path::Path,
    thread,
    time::{Duration, Instant},
};

const ENTRY_COUNT: usize = 20_000;

fn platform() -> Platform {
    Platform {
        os: "linux".into(),
        architecture: "amd64".into(),
        variant: None,
    }
}

fn digest(bytes: &[u8]) -> ArtifactDigest {
    format!("sha256:{}", hex::encode(Sha256::digest(bytes)))
        .parse()
        .unwrap()
}

fn private_dir() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o700)).unwrap();
    dir
}

fn request(id: OperationId, action: Action) -> Request {
    Request {
        version: VERSION,
        operation_id: id,
        action,
    }
}

fn image(store: &mut Store) -> ArtifactDigest {
    let layer_file = tempfile::NamedTempFile::new().unwrap();
    let mut tar = tar::Builder::new(layer_file.as_file());
    for index in 0..ENTRY_COUNT {
        let data = [b'x'; 1];
        let mut header = tar::Header::new_gnu();
        header.set_size(1);
        header.set_mode(0o600);
        header.set_cksum();
        tar.append_data(&mut header, format!("entry-{index:05}"), Cursor::new(data))
            .unwrap();
    }
    tar.finish().unwrap();
    drop(tar);
    let mut layer_file = layer_file.reopen().unwrap();
    let mut bytes = Vec::new();
    layer_file.read_to_end(&mut bytes).unwrap();
    let layer_digest = digest(&bytes);
    store
        .import_blob(&mut Cursor::new(&bytes), &layer_digest, bytes.len() as u64)
        .unwrap();
    let config = serde_json::to_vec(&serde_json::json!({
        "architecture": "amd64",
        "os": "linux",
        "rootfs": {"type": "layers", "diff_ids": [layer_digest]},
    }))
    .unwrap();
    let config_digest = digest(&config);
    store
        .import_blob(
            &mut Cursor::new(&config),
            &config_digest,
            config.len() as u64,
        )
        .unwrap();
    let manifest = serde_json::to_vec(&serde_json::json!({
        "schemaVersion": 2,
        "mediaType": "application/vnd.oci.image.manifest.v1+json",
        "config": {"mediaType": "application/vnd.oci.image.config.v1+json", "digest": config_digest, "size": config.len()},
        "layers": [{"mediaType": "application/vnd.oci.image.layer.v1.tar", "digest": layer_digest, "size": bytes.len()}],
    }))
    .unwrap();
    let manifest_digest = digest(&manifest);
    store
        .import_blob(
            &mut Cursor::new(&manifest),
            &manifest_digest,
            manifest.len() as u64,
        )
        .unwrap();
    store.admit_oci(&manifest_digest, &platform()).unwrap();
    manifest_digest
}

fn prepared_entries(path: &Path, id: &str) -> Option<usize> {
    Some(read_dir(path.join("prepared").join(id)).ok()?.count())
}

#[test]
#[ignore = "native SIGKILL prepared GC qualification"]
fn sigkill_prepared_gc_recovers_partial_tree() {
    for attempt in 0..2 {
        let store_root = private_dir();
        let runtime = private_dir();
        let (manifest, prepared_id) = {
            let mut store = Store::open(store_root.path(), Limits::default()).unwrap();
            let manifest = image(&mut store);
            let prepared = store.prepare(&manifest, &platform()).unwrap();
            (manifest, prepared)
        };
        let before = prepared_entries(store_root.path(), prepared_id.as_str()).unwrap();
        assert_eq!(before, ENTRY_COUNT);
        let (mut daemon, socket) = Daemon::spawn(store_root.path(), runtime.path());
        let operation_id = client::allocate(&socket).unwrap();
        let gc = request(operation_id.clone(), Action::Gc { max_entries: 1 });
        let worker_socket = socket.clone();
        let worker_gc = gc.clone();
        let worker = thread::spawn(move || client::call(&worker_socket, &worker_gc, None));
        let deadline = Instant::now() + Duration::from_secs(60);
        let mut observed = None;
        while Instant::now() < deadline && !worker.is_finished() {
            if let Some(count) = prepared_entries(store_root.path(), prepared_id.as_str()) {
                if count > 0 && count < before {
                    observed = Some(count);
                    break;
                }
            } else {
                // The effect may have completed between is_finished and this
                // observation. Treat disappearance as no observed window and
                // let the bounded retry decide whether it is reproducible.
                break;
            }
            thread::sleep(Duration::from_millis(2));
        }
        let Some(partial) = observed else {
            let result = worker.join().unwrap().unwrap().0.result;
            drop(daemon);
            if result.is_ok() && attempt == 0 {
                continue;
            }
            panic!("prepared GC completed before an observable partial tree: {result:?}");
        };
        assert!(partial < before);
        daemon.0.kill().unwrap();
        daemon.0.wait().unwrap();
        let _ = worker.join();

        let dir =
            cap_std::fs::Dir::open_ambient_dir(store_root.path(), cap_std::ambient_authority())
                .unwrap();
        let state = State::open(&dir).unwrap();
        let intent = state
            .get::<Prepared>("prepared", prepared_id.as_str())
            .unwrap()
            .unwrap();
        assert!(intent.phase == "gc_intent" || intent.phase == "deleting");
        let journal = state
            .get::<Operation>("operations", operation_id.as_str())
            .unwrap()
            .unwrap();
        assert_eq!(journal.phase, "intent");
        drop(state);

        let restarted_runtime = private_dir();
        let (daemon, restarted_socket) = Daemon::spawn(store_root.path(), restarted_runtime.path());
        let uncertain = client::call(&restarted_socket, &gc, None)
            .unwrap()
            .0
            .result
            .unwrap_err();
        assert!(uncertain.contains("outcome uncertain"), "{uncertain}");
        assert!(
            client::call(
                &restarted_socket,
                &request(
                    client::allocate(&restarted_socket).unwrap(),
                    Action::Unpin {
                        id: "after-crash".to_owned().try_into().unwrap()
                    }
                ),
                None
            )
            .unwrap()
            .0
            .result
            .is_ok()
        );
        drop(daemon);
        assert!(
            Store::open(store_root.path(), Limits::default())
                .unwrap()
                .open_blob(&manifest)
                .is_ok()
        );
        assert!(
            !store_root
                .path()
                .join("prepared")
                .join(prepared_id.as_str())
                .exists()
        );
        let state = State::open(&dir).unwrap();
        assert!(
            state
                .get::<Prepared>("prepared", prepared_id.as_str())
                .unwrap()
                .is_none()
        );
        let terminal: Operation = state
            .get("operations", operation_id.as_str())
            .unwrap()
            .unwrap();
        assert_eq!(terminal.phase, "failed");
        assert!(terminal.result.unwrap().contains("outcome uncertain"));
        assert!(
            fs::read_dir(store_root.path().join("prepared"))
                .unwrap()
                .next()
                .is_none()
        );
        drop(state);
        return;
    }
    unreachable!();
}
