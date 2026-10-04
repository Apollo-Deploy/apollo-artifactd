//! Requires an isolated HTTPS registry with authentication. No production endpoint.
mod support;
use apollo_artifactd::{
    Limits, Store,
    registry::{Credentials, Registry},
};
use std::{fs::File, os::unix::fs::PermissionsExt};
use support::*;

fn provider() -> Credentials {
    let path =
        std::env::var_os("ARTIFACTD_TEST_REGISTRY_CREDENTIALS").expect("private credential file");
    Credentials::read(Some(File::open(path).unwrap())).unwrap()
}
fn store() -> (tempfile::TempDir, Store) {
    let root = tempfile::tempdir().unwrap();
    std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let store = Store::open(root.path(), Limits::default()).unwrap();
    (root, store)
}

#[test]
#[ignore = "requires isolated authenticated HTTPS registry and private credential provider"]
fn https_registry_push_pull_verifies_graph_and_retries() {
    let destination =
        std::env::var("ARTIFACTD_TEST_REGISTRY_REFERENCE").expect("isolated registry reference");
    let registry = Registry::new().unwrap();
    let (_source_root, mut source) = store();
    let manifest = image(
        &mut source,
        "amd64",
        &[layer(&[("content", &vec![0x5a; 262144])])],
        false,
    );
    registry
        .push(&source, &manifest, &destination, provider())
        .unwrap();
    registry
        .push(&source, &manifest, &destination, provider())
        .unwrap();
    let pinned = format!("{}@{}", destination.rsplit_once(':').unwrap().0, manifest);
    let (_target_root, mut target) = store();
    registry
        .pull(&mut target, &pinned, &platform("amd64"), provider())
        .unwrap();
    assert_eq!(
        target
            .resolve(&manifest, &platform("amd64"))
            .unwrap()
            .digest,
        manifest
    );
    assert_eq!(
        target.status().unwrap()["bytes"],
        source.status().unwrap()["bytes"]
    );
    let before = target.status().unwrap();
    registry
        .pull(&mut target, &pinned, &platform("amd64"), provider())
        .unwrap();
    assert_eq!(target.status().unwrap(), before);
    let prepared = target.prepare(&manifest, &platform("amd64")).unwrap();
    target.lease("registry-rootfs", &manifest).unwrap();
    let fd = target.open_prepared(&prepared, "registry-rootfs").unwrap();
    assert_eq!(
        cap_std::fs::Dir::from_std_file(fd).read("content").unwrap(),
        vec![0x5a; 262144]
    );

    let (_rejected_root, mut rejected) = store();
    assert!(
        registry
            .pull(&mut rejected, &destination, &platform("amd64"), provider())
            .unwrap_err()
            .to_string()
            .contains("digest reference")
    );
    assert_eq!(rejected.status().unwrap()["blobs"], 0);
    let wrong = format!(
        "{}@{}",
        destination.rsplit_once(':').unwrap().0,
        digest(b"not the manifest")
    );
    assert!(
        registry
            .pull(&mut rejected, &wrong, &platform("amd64"), provider())
            .is_err()
    );
    assert_eq!(rejected.status().unwrap()["blobs"], 0);
    // Authentication failure must neither admit bytes nor echo credential data.
    let provider_path = std::env::var_os("ARTIFACTD_TEST_REGISTRY_CREDENTIALS").unwrap();
    let mut bad: serde_json::Value =
        serde_json::from_slice(&std::fs::read(provider_path).unwrap()).unwrap();
    bad["password"] = serde_json::json!("artifactd-invalid-password-sentinel");
    let mut bad_file = tempfile::NamedTempFile::new().unwrap();
    serde_json::to_writer(bad_file.as_file_mut(), &bad).unwrap();
    let bad_provider = Credentials::read(Some(File::open(bad_file.path()).unwrap())).unwrap();
    let failure = registry
        .pull(&mut rejected, &pinned, &platform("amd64"), bad_provider)
        .unwrap_err()
        .to_string();
    assert_eq!(failure, "registry request failed");
    assert_eq!(rejected.status().unwrap()["blobs"], 0);
}
