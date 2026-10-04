use apollo_artifactd::{Limits, Store};
use artifactd_protocol::ArtifactDigest;
use sha2::{Digest, Sha256};
use std::{
    io::{Cursor, Read},
    os::unix::fs::PermissionsExt,
};

fn fixture() -> (tempfile::TempDir, Store) {
    let root = tempfile::tempdir().unwrap();
    std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let store = Store::open(root.path(), Limits::default()).unwrap();
    (root, store)
}
fn digest(bytes: &[u8]) -> ArtifactDigest {
    format!("sha256:{}", hex::encode(Sha256::digest(bytes)))
        .parse()
        .unwrap()
}

#[test]
fn cas_admission_durability_and_references() {
    let (root, mut store) = fixture();
    let data = b"artifact payload";
    let d = digest(data);
    store
        .import_blob(&mut Cursor::new(data), &d, data.len() as u64)
        .unwrap();
    store
        .import_blob(&mut Cursor::new(data), &d, data.len() as u64)
        .unwrap();
    let mut read = Vec::new();
    store.open_blob(&d).unwrap().read_to_end(&mut read).unwrap();
    assert_eq!(read, data);
    store.pin("retention", &d).unwrap();
    store.lease("consumer", &d).unwrap();
    assert_eq!(store.gc(10).unwrap(), 0);
    store.unpin("retention").unwrap();
    assert_eq!(store.gc(10).unwrap(), 0);
    drop(store);
    let mut store = Store::open(root.path(), Limits::default()).unwrap();
    assert!(store.leased("consumer", &d).unwrap());
    store.release("consumer").unwrap();
    assert_eq!(store.gc(10).unwrap(), 1);
    assert!(store.open_blob(&d).is_err());
}

#[test]
fn incremental_gc_progresses_past_pinned_prefix_after_restart() {
    let (root, mut store) = fixture();
    let mut inputs = [b"first".as_slice(), b"second".as_slice()];
    inputs.sort_by_key(|bytes| digest(bytes));
    let pinned = digest(inputs[0]);
    let collectible = digest(inputs[1]);
    for bytes in inputs {
        store
            .import_blob(&mut Cursor::new(bytes), &digest(bytes), bytes.len() as u64)
            .unwrap();
    }
    store.pin("keep", &pinned).unwrap();
    assert_eq!(store.gc(1).unwrap(), 0);
    drop(store);
    let mut store = Store::open(root.path(), Limits::default()).unwrap();
    assert_eq!(store.gc(1).unwrap(), 1);
    assert!(store.open_blob(&collectible).is_err());
    assert!(store.open_blob(&pinned).is_ok());
}

#[test]
fn invalid_inputs_never_publish() {
    let (root, mut store) = fixture();
    for (data, d, size) in [
        (b"wrong".as_slice(), digest(b"right"), 5),
        (b"short".as_slice(), digest(b"short"), 6),
        (b"longer".as_slice(), digest(b"longer"), 5),
    ] {
        assert!(store.import_blob(&mut Cursor::new(data), &d, size).is_err());
        assert!(store.open_blob(&d).is_err());
        assert!(!root.path().join("blobs").join(d.hex()).exists());
    }
    assert_eq!(
        std::fs::read_dir(root.path().join("temp")).unwrap().count(),
        0
    );
}

#[test]
fn calculated_identity_streams_deduplicates_and_rejects_wrong_size() {
    let (root, mut store) = fixture();
    let data = vec![17; 131_073];
    let expected = digest(&data);
    assert_eq!(
        store
            .import_calculated(&mut Cursor::new(&data), data.len() as u64)
            .unwrap(),
        expected
    );
    assert_eq!(
        store
            .import_calculated(&mut Cursor::new(&data), data.len() as u64)
            .unwrap(),
        expected
    );
    assert!(
        store
            .import_calculated(&mut Cursor::new(b"invalid"), 8)
            .is_err()
    );
    assert_eq!(store.status().unwrap()["blobs"], 1);
    assert_eq!(
        std::fs::read_dir(root.path().join("temp")).unwrap().count(),
        0
    );
    drop(store);
    let store = Store::open(root.path(), Limits::default()).unwrap();
    let mut output = Vec::new();
    store
        .open_blob(&expected)
        .unwrap()
        .read_to_end(&mut output)
        .unwrap();
    assert_eq!(output, data);
}

#[test]
fn foreign_files_and_links_are_never_collected() {
    let (root, mut store) = fixture();
    let d = digest(b"owned");
    store
        .import_blob(&mut Cursor::new(b"owned"), &d, 5)
        .unwrap();
    let external = tempfile::NamedTempFile::new().unwrap();
    std::fs::remove_file(root.path().join("blobs").join(d.hex())).unwrap();
    std::os::unix::fs::symlink(external.path(), root.path().join("blobs").join(d.hex())).unwrap();
    assert!(store.gc(10).is_err());
    assert!(external.path().exists());
    assert!(
        root.path()
            .join("blobs")
            .join(d.hex())
            .symlink_metadata()
            .unwrap()
            .file_type()
            .is_symlink()
    );
}

#[test]
fn quota_and_exclusive_store_owner() {
    let (root, store) = fixture();
    assert!(Store::open(root.path(), Limits::default()).is_err());
    drop(store);
    let mut store = Store::open(
        root.path(),
        Limits {
            max_store: 4,
            max_objects: 1,
            max_temp_bytes: 4,
            ..Limits::default()
        },
    )
    .unwrap();
    assert!(
        store
            .import_blob(&mut Cursor::new(b"large"), &digest(b"large"), 5)
            .is_err()
    );
    let d = digest(b"full");
    store.import_blob(&mut Cursor::new(b"full"), &d, 4).unwrap();
    store.import_blob(&mut Cursor::new(b"full"), &d, 4).unwrap();
    assert!(store.import_blob(&mut Cursor::new(b"fake"), &d, 4).is_err());
    assert!(store.import_calculated(&mut Cursor::new(b""), 0).is_err());
    drop(store);
    let mut store = Store::open(
        root.path(),
        Limits {
            max_temp_bytes: 1,
            ..Limits::default()
        },
    )
    .unwrap();
    assert!(
        store
            .import_calculated(&mut Cursor::new(b"temp"), 4)
            .is_err()
    );
    store.import_blob(&mut Cursor::new(b"full"), &d, 4).unwrap();
}

proptest::proptest! {
    #[test]
    fn stored_identity_matches_bytes(bytes in proptest::collection::vec(proptest::num::u8::ANY,0..131072)) {
        let (_root,mut store) = fixture();
        let d = digest(&bytes);
        store.import_blob(&mut Cursor::new(&bytes),&d,bytes.len() as u64).unwrap();
        let mut output = Vec::new();
        store.open_blob(&d).unwrap().read_to_end(&mut output).unwrap();
        proptest::prop_assert_eq!(output,bytes);
    }
}

#[test]
fn legacy_sqlite_state_is_rejected_without_cleanup() {
    let (root, store) = fixture();
    drop(store);
    let foreign = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(foreign.path(), b"foreign database sidecar").unwrap();
    let sidecar = root.path().join("state.sqlite");
    std::os::unix::fs::symlink(foreign.path(), &sidecar).unwrap();
    assert!(Store::open(root.path(), Limits::default()).is_err());
    assert!(sidecar.symlink_metadata().unwrap().file_type().is_symlink());
    assert_eq!(
        std::fs::read(foreign.path()).unwrap(),
        b"foreign database sidecar"
    );
}
