#![cfg(target_os = "linux")]

#[path = "support/daemon.rs"]
mod daemon;

use artifactd_protocol::{Action, ArtifactDigest, OperationId, Platform, Request, VERSION, client};
use daemon::Daemon;
use sha2::{Digest, Sha256};
use std::{
    io::{Cursor, Seek, Write},
    os::unix::fs::PermissionsExt,
    path::Path,
};

fn digest(bytes: &[u8]) -> ArtifactDigest {
    format!("sha256:{}", hex::encode(Sha256::digest(bytes)))
        .parse()
        .unwrap()
}

fn platform() -> Platform {
    Platform {
        os: "linux".into(),
        architecture: "amd64".into(),
        variant: None,
    }
}

fn archive(tag: &str) -> (tempfile::NamedTempFile, ArtifactDigest) {
    let layer_data = format!("layer-{tag}").into_bytes();
    let mut layer_builder = tar::Builder::new(Vec::new());
    let mut layer_header = tar::Header::new_gnu();
    layer_header.set_size(layer_data.len() as u64);
    layer_header.set_mode(0o600);
    layer_header.set_cksum();
    layer_builder
        .append_data(&mut layer_header, "content", Cursor::new(&layer_data))
        .unwrap();
    let layer = layer_builder.into_inner().unwrap();
    let layer_digest = digest(&layer);
    let config = serde_json::to_vec(&serde_json::json!({
        "architecture": "amd64",
        "os": "linux",
        "rootfs": {"type": "layers", "diff_ids": [layer_digest]},
    }))
    .unwrap();
    let config_digest = digest(&config);
    let manifest = serde_json::to_vec(&serde_json::json!({
        "schemaVersion": 2,
        "mediaType": "application/vnd.oci.image.manifest.v1+json",
        "config": {
            "mediaType": "application/vnd.oci.image.config.v1+json",
            "digest": config_digest,
            "size": config.len(),
        },
        "layers": [{
            "mediaType": "application/vnd.oci.image.layer.v1.tar",
            "digest": layer_digest,
            "size": layer.len(),
        }],
    }))
    .unwrap();
    let manifest_digest = digest(&manifest);
    let index = serde_json::to_vec(&serde_json::json!({
        "schemaVersion": 2,
        "mediaType": "application/vnd.oci.image.index.v1+json",
        "manifests": [{
            "mediaType": "application/vnd.oci.image.manifest.v1+json",
            "digest": manifest_digest,
            "size": manifest.len(),
            "platform": {"os": "linux", "architecture": "amd64"},
        }],
    }))
    .unwrap();
    let index_digest = digest(&index);
    let mut builder = tar::Builder::new(Vec::new());
    let mut append = |name: String, bytes: &[u8]| {
        let mut header = tar::Header::new_gnu();
        header.set_size(bytes.len() as u64);
        header.set_mode(0o600);
        header.set_cksum();
        builder
            .append_data(&mut header, name, Cursor::new(bytes))
            .unwrap();
    };
    append("oci-layout".into(), br#"{"imageLayoutVersion":"1.0.0"}"#);
    append("index.json".into(), &index);
    append(format!("blobs/sha256/{}", layer_digest.hex()), &layer);
    append(format!("blobs/sha256/{}", config_digest.hex()), &config);
    append(format!("blobs/sha256/{}", manifest_digest.hex()), &manifest);
    let mut file = tempfile::NamedTempFile::new().unwrap();
    file.write_all(&builder.into_inner().unwrap()).unwrap();
    file.as_file_mut().rewind().unwrap();
    (file, index_digest)
}

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

fn ready_dir(path: &Path) {
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).unwrap();
}

fn verify(socket: &Path, digest: ArtifactDigest) -> bool {
    client::call(
        socket,
        &request(
            "verify-observation".to_owned().try_into().unwrap(),
            Action::Verify { digest },
        ),
        None,
    )
    .unwrap()
    .0
    .result
    .is_ok()
}

#[test]
fn daemon_oci_admission_pin_survives_replay_and_controls_gc() {
    let root = tempfile::tempdir().unwrap();
    let runtime = tempfile::tempdir().unwrap();
    ready_dir(root.path());
    ready_dir(runtime.path());
    let (first_archive, first_digest) = archive("first");
    let (second_archive, _second_digest) = archive("second");
    let pin: artifactd_protocol::PinId = "image-pin".to_owned().try_into().unwrap();
    let (daemon, socket) = Daemon::spawn(root.path(), runtime.path());

    let first_request = mutation(
        &socket,
        Action::ImportOciArchive {
            platform: platform(),
            pin: Some(pin.clone()),
        },
    );
    let first_result = client::call(&socket, &first_request, Some(first_archive.as_file()))
        .unwrap()
        .0
        .result
        .unwrap();
    assert_eq!(
        first_result["artifact_digest"],
        serde_json::json!(first_digest)
    );
    assert_eq!(first_result["pin_id"], serde_json::json!(pin));
    let direct_admission = client::call(
        &socket,
        &mutation(
            &socket,
            Action::ImportOci {
                digest: first_digest.clone(),
                platform: platform(),
                pin: Some(pin.clone()),
            },
        ),
        None,
    )
    .unwrap()
    .0
    .result
    .unwrap();
    assert_eq!(
        direct_admission["artifact_digest"],
        serde_json::json!(first_digest)
    );
    assert_eq!(direct_admission["pin_id"], serde_json::json!(pin));
    assert!(verify(&socket, first_digest.clone()));
    assert!(
        client::call(
            &socket,
            &mutation(&socket, Action::Gc { max_entries: 4096 }),
            None
        )
        .unwrap()
        .0
        .result
        .is_ok()
    );
    assert!(verify(&socket, first_digest.clone()));

    second_archive.as_file().rewind().unwrap();
    let conflict = client::call(
        &socket,
        &mutation(
            &socket,
            Action::ImportOciArchive {
                platform: platform(),
                pin: Some(pin.clone()),
            },
        ),
        Some(second_archive.as_file()),
    )
    .unwrap()
    .0
    .result;
    assert!(
        conflict
            .unwrap_err()
            .contains("reference identity conflict")
    );
    assert!(verify(&socket, first_digest.clone()));
    drop(daemon);

    let restart_runtime = tempfile::tempdir().unwrap();
    ready_dir(restart_runtime.path());
    let (daemon, restart_socket) = Daemon::spawn(root.path(), restart_runtime.path());
    first_archive.as_file().rewind().unwrap();
    let replay = client::call(
        &restart_socket,
        &first_request,
        Some(first_archive.as_file()),
    )
    .unwrap()
    .0
    .result
    .unwrap();
    assert_eq!(replay["artifact_digest"], serde_json::json!(first_digest));
    assert!(verify(&restart_socket, first_digest.clone()));
    client::call(
        &restart_socket,
        &mutation(&restart_socket, Action::Gc { max_entries: 4096 }),
        None,
    )
    .unwrap()
    .0
    .result
    .unwrap();
    assert!(verify(&restart_socket, first_digest.clone()));

    let unpin = client::call(
        &restart_socket,
        &mutation(&restart_socket, Action::Unpin { id: pin }),
        None,
    )
    .unwrap()
    .0
    .result;
    assert!(unpin.is_ok());
    client::call(
        &restart_socket,
        &mutation(&restart_socket, Action::Gc { max_entries: 4096 }),
        None,
    )
    .unwrap()
    .0
    .result
    .unwrap();
    assert!(!verify(&restart_socket, first_digest));
    drop(daemon);
}
