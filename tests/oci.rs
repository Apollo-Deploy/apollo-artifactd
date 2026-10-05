mod support;
use apollo_artifactd::{Limits, Store, oci::facts, state::State};
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
    let lease = store.lease(&manifest).unwrap();
    let fd = store.open_prepared(&id, lease.as_str()).unwrap();
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
    store.release(lease.as_str()).unwrap();
    assert!(store.gc(100).unwrap() > 0);
}

#[test]
fn admitted_oci_facts_include_verified_descriptor_receipt_data() {
    let root = tempfile::tempdir().unwrap();
    std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let mut store = Store::open(root.path(), Limits::default()).unwrap();
    let layer_bytes = layer(&[("file", b"payload")]);
    let manifest_digest = image(
        &mut store,
        "amd64",
        std::slice::from_ref(&layer_bytes),
        false,
    );
    let resolved = store.resolve(&manifest_digest, &platform("amd64")).unwrap();
    let value = facts(&manifest_digest, &resolved, &platform("amd64"));
    let mut manifest_bytes = Vec::new();
    store
        .open_blob(&manifest_digest)
        .unwrap()
        .read_to_end(&mut manifest_bytes)
        .unwrap();
    let manifest: serde_json::Value = serde_json::from_slice(&manifest_bytes).unwrap();
    assert_eq!(value["verified"], true);
    assert_eq!(value["artifact_digest"], manifest_digest.to_string());
    assert_eq!(value["manifest_digest"], manifest_digest.to_string());
    assert_eq!(value["manifest_size"], manifest_bytes.len());
    assert_eq!(
        value["config"]["media_type"],
        "application/vnd.oci.image.config.v1+json"
    );
    assert_eq!(value["config"]["digest"], manifest["config"]["digest"]);
    assert_eq!(value["config"]["size"], manifest["config"]["size"]);
    assert_eq!(value["layers"].as_array().unwrap().len(), 1);
    assert_eq!(
        value["layers"][0]["digest"],
        manifest["layers"][0]["digest"]
    );
    assert_eq!(
        value["layers"][0]["media_type"],
        "application/vnd.oci.image.layer.v1.tar"
    );
    assert_eq!(value["layers"][0]["size"], layer_bytes.len());
    assert_eq!(value["layer_digests"][0], manifest["layers"][0]["digest"]);
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
    drop(store);
    let dir =
        cap_std::fs::Dir::open_ambient_dir(root.path(), cap_std::ambient_authority()).unwrap();
    let db = State::open(&dir).unwrap();
    db.remove("edges", manifest.as_str()).unwrap();
    drop(db);
    assert!(Store::open(root.path(), Limits::default()).is_err());
    let db = State::open(&dir).unwrap();
    for (key, _) in db
        .scan::<apollo_artifactd::state::Root>("roots", None, 4096)
        .unwrap()
    {
        db.remove("roots", &key).unwrap();
    }
    drop(db);
    assert!(Store::open(root.path(), Limits::default()).is_err());
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
        let path = format!("./{path}");
        archive
            .append_pax_extensions([("path", path.as_bytes())])
            .unwrap();
        let mut h = tar::Header::new_gnu();
        h.set_size(data.len() as u64);
        h.set_mode(0o600);
        h.set_cksum();
        archive
            .append_data(&mut h, &path, Cursor::new(data))
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
    let mut bad_prefix = tar::Builder::new(Vec::new());
    let mut bad_directory = tar::Header::new_gnu();
    bad_directory.set_entry_type(tar::EntryType::Directory);
    bad_directory.set_size(1);
    bad_directory.set_cksum();
    bad_prefix
        .append_data(&mut bad_directory, "blobs", &b"x"[..])
        .unwrap();
    let bad_prefix = bad_prefix.into_inner().unwrap();
    let mut bad_archive = bad_prefix[..bad_prefix.len() - 1024].to_vec();
    bad_archive.extend_from_slice(&archive);
    let bad_root = tempfile::tempdir().unwrap();
    std::fs::set_permissions(bad_root.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let mut bad_store = Store::open(bad_root.path(), Limits::default()).unwrap();
    let bad_error = bad_store
        .import_oci_archive(Cursor::new(bad_archive), &platform("arm64"))
        .unwrap_err();
    assert!(
        bad_error
            .to_string()
            .contains("unexpected archive directory")
    );

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

#[test]
fn malformed_oci_archive_pax_is_rejected() {
    let root = tempfile::tempdir().unwrap();
    std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let mut store = Store::open(root.path(), Limits::default()).unwrap();
    let mut archive = tar::Builder::new(Vec::new());
    archive
        .append_pax_extensions([
            ("path", b"oci-layout".as_slice()),
            ("path", b"duplicate".as_slice()),
        ])
        .unwrap();
    let mut header = tar::Header::new_gnu();
    header.set_size(0);
    header.set_cksum();
    archive.append(&header, std::io::empty()).unwrap();
    assert!(
        store
            .import_oci_archive(
                Cursor::new(archive.into_inner().unwrap()),
                &platform("arm64")
            )
            .unwrap_err()
            .to_string()
            .contains("duplicate PAX key")
    );
}

#[test]
fn canonical_duplicate_layout_paths_are_rejected() {
    let root = tempfile::tempdir().unwrap();
    std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let mut store = Store::open(root.path(), Limits::default()).unwrap();
    let mut archive = tar::Builder::new(Vec::new());
    for path in ["blobs", "./blobs"] {
        let mut header = tar::Header::new_gnu();
        header.set_entry_type(tar::EntryType::Directory);
        header.set_size(0);
        header.set_cksum();
        archive
            .append_data(&mut header, path, std::io::empty())
            .unwrap();
    }
    let error = store
        .import_oci_archive(
            Cursor::new(archive.into_inner().unwrap()),
            &platform("arm64"),
        )
        .unwrap_err();
    assert!(
        error.to_string().contains("archive duplicate/entry limit"),
        "{error:#}"
    );
}
