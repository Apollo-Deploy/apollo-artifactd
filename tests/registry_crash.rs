#![cfg(target_os = "linux")]

//! Real registry pull interruption coverage.  This remains ignored because it
//! needs the private authenticated HTTPS distribution fixture.
#[path = "support/daemon.rs"]
mod daemon;

use apollo_artifactd::{
    Limits, Store,
    registry::{Credentials, Registry},
    state::{Operation, State},
};
use artifactd_protocol::{Action, ArtifactDigest, Platform, Request, VERSION, client};
use sha2::{Digest, Sha256};
use std::{
    fs::{File, read_dir},
    io::{Read, Seek, SeekFrom},
    os::unix::fs::PermissionsExt,
    process::Child,
    thread,
    time::{Duration, Instant},
};

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

fn import_bytes(store: &mut Store, bytes: &[u8]) -> ArtifactDigest {
    let digest = digest(bytes);
    store
        .import_blob(
            &mut std::io::Cursor::new(bytes),
            &digest,
            bytes.len() as u64,
        )
        .unwrap();
    digest
}

fn layer() -> (tempfile::NamedTempFile, ArtifactDigest, u64) {
    let file = tempfile::NamedTempFile::new().unwrap();
    let size = 256 * 1024 * 1024u64;
    let mut tar = tar::Builder::new(file.as_file());
    let mut header = tar::Header::new_gnu();
    header.set_size(size);
    header.set_mode(0o600);
    header.set_cksum();
    tar.append_data(&mut header, "payload", std::io::repeat(0).take(size))
        .unwrap();
    tar.finish().unwrap();
    drop(tar);
    let mut input = file.reopen().unwrap();
    let mut hash = Sha256::new();
    let mut buf = [0u8; 64 * 1024];
    let mut length = 0u64;
    loop {
        let n = input.read(&mut buf).unwrap();
        if n == 0 {
            break;
        }
        hash.update(&buf[..n]);
        length += n as u64;
    }
    (
        file,
        format!("sha256:{}", hex::encode(hash.finalize()))
            .parse()
            .unwrap(),
        length,
    )
}

fn image(
    store: &mut Store,
    layer_file: &mut File,
    layer_digest: &ArtifactDigest,
    layer_size: u64,
) -> ArtifactDigest {
    layer_file.seek(SeekFrom::Start(0)).unwrap();
    store
        .import_blob(layer_file, layer_digest, layer_size)
        .unwrap();
    let config = serde_json::to_vec(&serde_json::json!({
        "schemaVersion": 2, "architecture": "amd64", "os": "linux",
        "rootfs": {"type": "layers", "diff_ids": [layer_digest]},
    }))
    .unwrap();
    let config_digest = import_bytes(store, &config);
    let manifest = serde_json::to_vec(&serde_json::json!({
        "schemaVersion": 2,
        "mediaType": "application/vnd.oci.image.manifest.v1+json",
        "config": {"mediaType": "application/vnd.oci.image.config.v1+json", "digest": config_digest, "size": config.len()},
        "layers": [{"mediaType": "application/vnd.oci.image.layer.v1.tar", "digest": layer_digest, "size": layer_size}],
    })).unwrap();
    let manifest_digest = import_bytes(store, &manifest);
    store.admit_oci(&manifest_digest, &platform()).unwrap();
    manifest_digest
}

fn private_dir() -> tempfile::TempDir {
    let root = tempfile::tempdir().unwrap();
    std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    root
}

fn request(id: artifactd_protocol::OperationId, action: Action) -> Request {
    Request {
        version: VERSION,
        operation_id: id,
        action,
    }
}

fn stop(child: &mut Child) {
    let _ = child.kill();
    let _ = child.wait();
}

#[test]
#[ignore = "requires isolated authenticated HTTPS registry and private credential provider"]
fn sigkill_registry_pull_retains_uncertain_operation_without_publishing_partial_blob() {
    let destination = std::env::var("ARTIFACTD_TEST_REGISTRY_REFERENCE").unwrap();
    let credential_path = std::env::var("ARTIFACTD_TEST_REGISTRY_CREDENTIALS").unwrap();
    let source_root = private_dir();
    let mut source = Store::open(source_root.path(), Limits::default()).unwrap();
    let (mut layer_file, layer_digest, layer_size) = layer();
    let manifest = image(
        &mut source,
        layer_file.as_file_mut(),
        &layer_digest,
        layer_size,
    );
    let registry = Registry::new().unwrap();
    registry
        .push(
            &source,
            &manifest,
            &destination,
            Credentials::read(Some(File::open(&credential_path).unwrap())).unwrap(),
        )
        .unwrap();
    drop(source);

    let target_root = private_dir();
    let target_runtime = private_dir();
    let (mut daemon, socket) = daemon::Daemon::spawn(target_root.path(), target_runtime.path());
    let base = destination
        .rsplit_once(':')
        .map(|(base, _)| base)
        .unwrap_or(&destination);
    let pinned = format!("{base}@{manifest}");
    let operation_id = client::allocate(&socket).unwrap();
    let pull = request(
        operation_id.clone(),
        Action::Pull {
            reference: pinned.clone(),
            platform: platform(),
        },
    );
    let worker_request = pull.clone();
    let pull_socket = socket.clone();
    let pull_credentials = File::open(&credential_path).unwrap();
    let worker =
        thread::spawn(move || client::call(&pull_socket, &worker_request, Some(&pull_credentials)));
    let deadline = Instant::now() + Duration::from_secs(180);
    let mut partial = None;
    while Instant::now() < deadline {
        let candidates: Vec<_> = read_dir(target_root.path().join("temp"))
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| entry.file_name().to_string_lossy().ends_with(".part"))
            .collect();
        if let Some(entry) = candidates.first()
            && let Ok(metadata) = entry.metadata()
        {
            let size = metadata.len();
            if size > 4 * 1024 * 1024 && size < layer_size {
                assert!(
                    !target_root
                        .path()
                        .join("blobs")
                        .join(layer_digest.hex())
                        .exists()
                );
                partial = Some((entry.path(), size));
                break;
            }
        }
        if worker.is_finished() {
            break;
        }
        thread::sleep(Duration::from_millis(2));
    }
    let (partial_path, partial_size) =
        partial.expect("pull completed before an observable partial CAS file");
    assert!(partial_size < layer_size);
    stop(&mut daemon.0);
    let _ = worker.join();
    assert!(partial_path.exists());
    assert!(
        !target_root
            .path()
            .join("blobs")
            .join(layer_digest.hex())
            .exists()
    );

    let dir = cap_std::fs::Dir::open_ambient_dir(target_root.path(), cap_std::ambient_authority())
        .unwrap();
    let state = State::open(&dir).unwrap();
    let intent = state
        .get::<Operation>("operations", operation_id.as_str())
        .unwrap()
        .unwrap();
    assert_eq!(intent.phase, "intent");
    assert_eq!(
        state
            .get::<Operation>("registry", operation_id.as_str())
            .unwrap(),
        Some(intent.clone())
    );
    drop(state);

    let restarted_runtime = private_dir();
    let (mut restarted, restarted_socket) =
        daemon::Daemon::spawn(target_root.path(), restarted_runtime.path());
    let retry = client::call(
        &restarted_socket,
        &pull,
        Some(&File::open(&credential_path).unwrap()),
    )
    .unwrap()
    .0;
    assert_eq!(
        retry.result.unwrap_err(),
        "operation interrupted; outcome uncertain; inspect state before retrying with a new token"
    );
    assert!(
        !target_root
            .path()
            .join("blobs")
            .join(layer_digest.hex())
            .exists()
    );
    assert!(
        read_dir(target_root.path().join("temp"))
            .unwrap()
            .all(|entry| !entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .ends_with(".part"))
    );

    let fresh_id = client::allocate(&restarted_socket).unwrap();
    let fresh = request(
        fresh_id.clone(),
        Action::Pull {
            reference: pinned,
            platform: platform(),
        },
    );
    let response = client::call(
        &restarted_socket,
        &fresh,
        Some(&File::open(&credential_path).unwrap()),
    )
    .unwrap()
    .0;
    assert!(
        response.result.is_ok(),
        "fresh pull failed: {:?}",
        response.result
    );
    assert!(
        target_root
            .path()
            .join("blobs")
            .join(layer_digest.hex())
            .exists()
    );
    stop(&mut restarted.0);
    let state = State::open(&dir).unwrap();
    let completed = state
        .get::<Operation>("operations", fresh_id.as_str())
        .unwrap()
        .unwrap();
    assert_eq!(completed.phase, "complete");
    assert_eq!(
        state
            .get::<Operation>("registry", fresh_id.as_str())
            .unwrap(),
        Some(completed)
    );
}
