#![cfg(target_os = "linux")]

#[path = "support/daemon.rs"]
mod daemon;

use apollo_artifactd::state::{Operation, State};
use artifactd_protocol::{
    Action, ArtifactDigest, Platform, PreparedDigest, Request, VERSION, client, client::call,
};
use daemon::Daemon;
use sha2::{Digest, Sha256};
use std::{
    fs::File,
    io::{Cursor, Seek, SeekFrom},
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

fn digest(bytes: &[u8]) -> ArtifactDigest {
    format!("sha256:{}", hex::encode(Sha256::digest(bytes)))
        .parse()
        .unwrap()
}

fn archive() -> (tempfile::NamedTempFile, ArtifactDigest) {
    let layer = {
        let mut tar = tar::Builder::new(Vec::new());
        let data = b"artifactd api registry";
        let mut header = tar::Header::new_gnu();
        header.set_size(data.len() as u64);
        header.set_mode(0o600);
        header.set_cksum();
        tar.append_data(&mut header, "content", Cursor::new(data))
            .unwrap();
        tar.into_inner().unwrap()
    };
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
    let mut archive = tar::Builder::new(Vec::new());
    let mut append = |name: &str, bytes: &[u8]| {
        let mut header = tar::Header::new_gnu();
        header.set_size(bytes.len() as u64);
        header.set_mode(0o600);
        header.set_cksum();
        archive
            .append_data(&mut header, name, Cursor::new(bytes))
            .unwrap();
    };
    append("oci-layout", br#"{"imageLayoutVersion":"1.0.0"}"#);
    append("index.json", &index);
    append(&format!("blobs/sha256/{}", layer_digest.hex()), &layer);
    append(&format!("blobs/sha256/{}", config_digest.hex()), &config);
    append(
        &format!("blobs/sha256/{}", manifest_digest.hex()),
        &manifest,
    );
    let mut file = tempfile::NamedTempFile::new().unwrap();
    std::io::Write::write_all(file.as_file_mut(), &archive.into_inner().unwrap()).unwrap();
    file.as_file_mut().seek(SeekFrom::Start(0)).unwrap();
    (file, index_digest)
}

fn private_credentials(path: &Path) -> File {
    let file = File::open(path).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).unwrap();
    file
}

fn pinned(reference: &str, digest: &ArtifactDigest) -> String {
    if let Some(at) = reference.rfind('@') {
        return format!("{}@{}", &reference[..at], digest);
    }
    let colon = reference.rfind(':').filter(|index| {
        reference[*index + 1..].chars().all(|c| c != '/')
            && reference[..*index]
                .rfind('/')
                .is_none_or(|slash| slash < *index)
    });
    match colon {
        Some(index) => format!("{}@{}", &reference[..index], digest),
        None => format!("{}@{}", reference, digest),
    }
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
    let credentials = private_credentials(Path::new(&credential_path));
    let pulled = call(
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
            },
        ),
        Some(&credentials),
    )
    .unwrap()
    .0
    .result
    .unwrap();
    assert_eq!(pulled["artifact_digest"], serde_json::json!(root));
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
