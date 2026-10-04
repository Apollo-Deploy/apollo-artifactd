mod support;
use apollo_artifactd::{Limits, Store};
use std::{
    io::{Cursor, Read},
    os::unix::fs::PermissionsExt,
};
use support::*;

#[test]
fn oci_preparation_verifies_diffid_and_whiteouts() {
    let root = tempfile::tempdir().unwrap();
    std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let mut store = Store::open(root.path(), Limits::default()).unwrap();
    let a = layer(&[("removed", b"old"), ("dir/old", b"old"), ("keep", b"base")]);
    let b = layer(&[
        (".wh.removed", b""),
        ("dir/.wh..wh..opq", b""),
        ("dir/new", b"new"),
    ]);
    let manifest = image(&mut store, "arm64", &[a, b], false);
    store.pin("test", &manifest).unwrap();
    let platform = platform("arm64");
    let id = store.prepare(&manifest, &platform).unwrap();
    store.lease("rootfs", &manifest).unwrap();
    let fd = store.open_prepared(&id, "rootfs").unwrap();
    let dir = cap_std::fs::Dir::from_std_file(fd);
    assert!(!dir.try_exists("removed").unwrap());
    assert!(!dir.try_exists("dir/old").unwrap());
    let mut value = String::new();
    dir.open("dir/new")
        .unwrap()
        .read_to_string(&mut value)
        .unwrap();
    assert_eq!(value, "new");
    assert_eq!(id, store.prepare(&manifest, &platform).unwrap());
    assert_eq!(store.gc(100).unwrap(), 0);
    store.unpin("test").unwrap();
    store.release("rootfs").unwrap();
    assert!(store.gc(100).unwrap() > 0);
}

#[test]
fn diffid_mismatch_and_symlink_escape_fail_preparation() {
    let root = tempfile::tempdir().unwrap();
    std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let mut store = Store::open(root.path(), Limits::default()).unwrap();
    let manifest = image(&mut store, "amd64", &[layer(&[("file", b"data")])], true);
    assert!(store.prepare(&manifest, &platform("amd64")).is_err());
    drop(store);
    let mut store = Store::open(root.path(), Limits::default()).unwrap();
    let mut builder = tar::Builder::new(Vec::new());
    let mut header = tar::Header::new_gnu();
    header.set_entry_type(tar::EntryType::Symlink);
    header.set_size(0);
    header.set_mode(0o777);
    header.set_link_name("../../escape").unwrap();
    header.set_cksum();
    builder
        .append_data(&mut header, "link", std::io::empty())
        .unwrap();
    let manifest = image(&mut store, "amd64", &[builder.into_inner().unwrap()], false);
    assert!(store.prepare(&manifest, &platform("amd64")).is_err());
}

#[test]
fn multi_platform_and_descriptor_substitution() {
    let root = tempfile::tempdir().unwrap();
    std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let mut store = Store::open(root.path(), Limits::default()).unwrap();
    let amd = image(&mut store, "amd64", &[], false);
    let arm = image(&mut store, "arm64", &[], false);
    let descriptor = |d: &artifactd_protocol::ArtifactDigest, arch: &str| serde_json::json!({"mediaType":MANIFEST,"digest":d,"size":store.open_blob(d).unwrap().metadata().unwrap().len(),"platform":{"os":"linux","architecture":arch}});
    let index = serde_json::to_vec(&serde_json::json!({"schemaVersion":2,"mediaType":"application/vnd.oci.image.index.v1+json","manifests":[descriptor(&amd,"amd64"),descriptor(&arm,"arm64")]})).unwrap();
    let digest = import(&mut store, &index);
    store.admit_oci(&digest, &platform("arm64")).unwrap();
    assert_eq!(
        store.resolve(&digest, &platform("amd64")).unwrap().digest,
        amd
    );
    let mut index: serde_json::Value = serde_json::from_slice(&index).unwrap();
    index["manifests"][0]["platform"]["architecture"] = serde_json::json!("arm64");
    let substituted = import(&mut store, &serde_json::to_vec(&index).unwrap());
    assert!(store.admit_oci(&substituted, &platform("arm64")).is_err());
}

#[test]
fn malicious_layout_archive_never_unpacks() {
    let root = tempfile::tempdir().unwrap();
    std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let mut store = Store::open(root.path(), Limits::default()).unwrap();
    let mut builder = tar::Builder::new(Vec::new());
    let mut header = tar::Header::new_gnu();
    header.set_size(0);
    header.set_mode(0o600);
    header.set_entry_type(tar::EntryType::Symlink);
    header.set_link_name("/etc/passwd").unwrap();
    header.set_cksum();
    builder
        .append_data(&mut header, "blobs/sha256/evil", std::io::empty())
        .unwrap();
    assert!(
        store
            .import_oci_archive(
                Cursor::new(builder.into_inner().unwrap()),
                &platform("arm64")
            )
            .is_err()
    );
}

#[test]
fn corrupt_reachability_cannot_authorize_gc() {
    let root = tempfile::tempdir().unwrap();
    std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let mut store = Store::open(root.path(), Limits::default()).unwrap();
    let manifest = image(
        &mut store,
        "amd64",
        &[layer(&[("keep", b"payload")])],
        false,
    );
    store.pin("keep", &manifest).unwrap();
    let db = rusqlite::Connection::open(root.path().join("state.sqlite")).unwrap();
    db.execute("DELETE FROM edges WHERE parent=?1", [manifest.as_str()])
        .unwrap();
    assert!(store.gc(100).is_err());
    assert_eq!(store.status().unwrap()["blobs"], 3);
    db.execute("DELETE FROM roots", []).unwrap();
    assert!(store.gc(100).is_err());
    assert_eq!(store.status().unwrap()["blobs"], 3);
}

#[test]
fn valid_archive_admission_and_missing_terminator() {
    let source = tempfile::tempdir().unwrap();
    std::fs::set_permissions(source.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let mut source_store = Store::open(source.path(), Limits::default()).unwrap();
    let manifest = image(
        &mut source_store,
        "arm64",
        &[layer(&[("file", b"payload")])],
        false,
    );
    let index = serde_json::to_vec(&serde_json::json!({"schemaVersion":2,"mediaType":"application/vnd.oci.image.index.v1+json","manifests":[{"mediaType":MANIFEST,"digest":manifest,"size":source_store.open_blob(&manifest).unwrap().metadata().unwrap().len()}]})).unwrap();
    let mut archive = tar::Builder::new(Vec::new());
    let mut append = |path: &str, data: &[u8]| {
        let mut h = tar::Header::new_gnu();
        h.set_size(data.len() as u64);
        h.set_mode(0o600);
        h.set_cksum();
        archive
            .append_data(&mut h, path, Cursor::new(data))
            .unwrap();
    };
    append("oci-layout", br#"{"imageLayoutVersion":"1.0.0"}"#);
    append("index.json", &index);
    for entry in std::fs::read_dir(source.path().join("blobs")).unwrap() {
        let entry = entry.unwrap();
        append(
            &format!("blobs/sha256/{}", entry.file_name().to_str().unwrap()),
            &std::fs::read(entry.path()).unwrap(),
        );
    }
    let archive = archive.into_inner().unwrap();
    for complete in [false, true] {
        let root = tempfile::tempdir().unwrap();
        std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        let mut store = Store::open(root.path(), Limits::default()).unwrap();
        let input = if complete {
            &archive[..]
        } else {
            &archive[..archive.len() - 1024]
        };
        let result = store.import_oci_archive(Cursor::new(input), &platform("arm64"));
        assert_eq!(result.is_ok(), complete);
        if complete {
            assert_eq!(
                store
                    .resolve(&digest(&index), &platform("arm64"))
                    .unwrap()
                    .digest,
                manifest
            );
        }
    }
}
