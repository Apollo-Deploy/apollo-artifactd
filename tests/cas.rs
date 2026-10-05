use apollo_artifactd::{
    Limits, Store,
    state::{Imported, State},
};
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
    let lease = store.lease(&d).unwrap();
    assert_eq!(store.gc(10).unwrap(), 0);
    store.unpin("retention").unwrap();
    assert_eq!(store.gc(10).unwrap(), 0);
    drop(store);
    let mut store = Store::open(root.path(), Limits::default()).unwrap();
    assert!(store.leased(lease.as_str(), &d).unwrap());
    store.release(lease.as_str()).unwrap();
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
fn corrupt_recorded_blob_is_repaired_without_losing_pin_protection() {
    let (root, mut store) = fixture();
    let data = b"repairable payload";
    let d = digest(data);
    store
        .import_blob(&mut Cursor::new(data), &d, data.len() as u64)
        .unwrap();
    store.pin("repair-pin", &d).unwrap();
    let path = root.path().join("blobs").join(d.hex());
    std::fs::remove_file(&path).unwrap();
    std::fs::write(&path, b"corrupt bytes").unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o400)).unwrap();
    assert!(store.open_blob(&d).is_err());
    store
        .import_blob(&mut Cursor::new(data), &d, data.len() as u64)
        .unwrap();
    let mut output = Vec::new();
    store
        .open_blob(&d)
        .unwrap()
        .read_to_end(&mut output)
        .unwrap();
    assert_eq!(output, data);
    assert_eq!(store.gc(10).unwrap(), 0);
}

#[test]
fn wrong_repair_input_does_not_replace_corrupt_blob() {
    let (root, mut store) = fixture();
    let data = b"repair target";
    let d = digest(data);
    store
        .import_blob(&mut Cursor::new(data), &d, data.len() as u64)
        .unwrap();
    let path = root.path().join("blobs").join(d.hex());
    std::fs::remove_file(&path).unwrap();
    std::fs::write(&path, b"corrupt bytes").unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o400)).unwrap();
    assert!(
        store
            .import_blob(&mut Cursor::new(b"wrong repair"), &d, data.len() as u64)
            .is_err()
    );
    assert!(store.open_blob(&d).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), b"corrupt bytes");
}

#[test]
fn unrecorded_digest_collision_is_preserved() {
    let (root, mut store) = fixture();
    let data = b"unrecorded target";
    let d = digest(data);
    let path = root.path().join("blobs").join(d.hex());
    std::fs::write(&path, b"foreign corrupt bytes").unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o400)).unwrap();
    assert!(
        store
            .import_blob(&mut Cursor::new(data), &d, data.len() as u64)
            .is_err()
    );
    assert_eq!(std::fs::read(&path).unwrap(), b"foreign corrupt bytes");
}

#[test]
fn repair_reconciles_after_publish_effect_window() {
    let (root, mut store) = fixture();
    let data = b"recovery repair payload";
    let d = digest(data);
    store
        .import_blob(&mut Cursor::new(data), &d, data.len() as u64)
        .unwrap();
    drop(store);

    let final_path = root.path().join("blobs").join(d.hex());
    std::fs::remove_file(&final_path).unwrap();
    std::fs::write(&final_path, b"corrupt final").unwrap();
    std::fs::set_permissions(&final_path, std::fs::Permissions::from_mode(0o400)).unwrap();

    let id = uuid::Uuid::new_v4().to_string();
    let temp = format!("{id}.part");
    let temp_path = root.path().join("temp").join(&temp);
    std::fs::write(&temp_path, data).unwrap();
    std::fs::set_permissions(&temp_path, std::fs::Permissions::from_mode(0o400)).unwrap();
    let dir =
        cap_std::fs::Dir::open_ambient_dir(root.path(), cap_std::ambient_authority()).unwrap();
    let db = State::open(&dir).unwrap();
    db.put(
        "imports",
        &id,
        &Imported {
            temp,
            digest: Some(d.clone()),
            size: data.len() as u64,
        },
    )
    .unwrap();
    drop(db);

    let recovered = Store::open(root.path(), Limits::default()).unwrap();
    let mut output = Vec::new();
    recovered
        .open_blob(&d)
        .unwrap()
        .read_to_end(&mut output)
        .unwrap();
    assert_eq!(output, data);
    drop(recovered);
    let db = State::open(&dir).unwrap();
    assert_eq!(db.count("imports").unwrap(), 0);
    assert_eq!(
        std::fs::read_dir(root.path().join("temp")).unwrap().count(),
        0
    );
}

#[test]
fn repair_reconciles_after_publish_before_database_commit() {
    let (root, mut store) = fixture();
    let data = b"post rename recovery payload";
    let d = digest(data);
    store
        .import_blob(&mut Cursor::new(data), &d, data.len() as u64)
        .unwrap();
    drop(store);

    let final_path = root.path().join("blobs").join(d.hex());
    std::fs::remove_file(&final_path).unwrap();
    std::fs::write(&final_path, b"corrupt final").unwrap();
    std::fs::set_permissions(&final_path, std::fs::Permissions::from_mode(0o400)).unwrap();
    std::fs::remove_file(&final_path).unwrap();
    std::fs::write(&final_path, data).unwrap();
    std::fs::set_permissions(&final_path, std::fs::Permissions::from_mode(0o400)).unwrap();

    let id = uuid::Uuid::new_v4().to_string();
    let dir =
        cap_std::fs::Dir::open_ambient_dir(root.path(), cap_std::ambient_authority()).unwrap();
    let db = State::open(&dir).unwrap();
    db.put(
        "imports",
        &id,
        &Imported {
            temp: format!("{id}.part"),
            digest: Some(d.clone()),
            size: data.len() as u64,
        },
    )
    .unwrap();
    drop(db);

    let recovered = Store::open(root.path(), Limits::default()).unwrap();
    let mut output = Vec::new();
    recovered
        .open_blob(&d)
        .unwrap()
        .read_to_end(&mut output)
        .unwrap();
    assert_eq!(output, data);
    drop(recovered);
    let db = State::open(&dir).unwrap();
    assert_eq!(db.count("imports").unwrap(), 0);
    assert_eq!(db.count("blobs").unwrap(), 1);
}

#[test]
fn recovery_preserves_unrecorded_digest_collision() {
    let (root, store) = fixture();
    drop(store);
    let data = b"unrecorded recovery target";
    let d = digest(data);
    let final_path = root.path().join("blobs").join(d.hex());
    std::fs::write(&final_path, b"foreign corrupt bytes").unwrap();
    std::fs::set_permissions(&final_path, std::fs::Permissions::from_mode(0o400)).unwrap();
    let id = uuid::Uuid::new_v4().to_string();
    let temp = format!("{id}.part");
    std::fs::write(root.path().join("temp").join(&temp), data).unwrap();
    std::fs::set_permissions(
        root.path().join("temp").join(&temp),
        std::fs::Permissions::from_mode(0o400),
    )
    .unwrap();
    let dir =
        cap_std::fs::Dir::open_ambient_dir(root.path(), cap_std::ambient_authority()).unwrap();
    let db = State::open(&dir).unwrap();
    db.put(
        "imports",
        &id,
        &Imported {
            temp: temp.clone(),
            digest: Some(d.clone()),
            size: data.len() as u64,
        },
    )
    .unwrap();
    drop(db);
    assert!(Store::open(root.path(), Limits::default()).is_err());
    assert_eq!(
        std::fs::read(&final_path).unwrap(),
        b"foreign corrupt bytes"
    );
    assert_eq!(
        std::fs::read(root.path().join("temp").join(&temp)).unwrap(),
        data
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
    assert!(
        store
            .import_blob(&mut Cursor::new(b"owned"), &d, 5)
            .is_err()
    );
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
            max_store: 8,
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
    let full_path = root.path().join("blobs").join(d.hex());
    std::fs::remove_file(&full_path).unwrap();
    std::fs::write(&full_path, b"bad!").unwrap();
    std::fs::set_permissions(&full_path, std::fs::Permissions::from_mode(0o400)).unwrap();
    store.import_blob(&mut Cursor::new(b"full"), &d, 4).unwrap();
    assert!(store.import_blob(&mut Cursor::new(b"fake"), &d, 4).is_err());
    assert!(store.import_calculated(&mut Cursor::new(b""), 0).is_err());
    drop(store);
    let mut constrained = Store::open(
        root.path(),
        Limits {
            max_store: 4,
            max_objects: 1,
            ..Limits::default()
        },
    )
    .unwrap();
    std::fs::remove_file(&full_path).unwrap();
    std::fs::write(&full_path, b"bad!").unwrap();
    std::fs::set_permissions(&full_path, std::fs::Permissions::from_mode(0o400)).unwrap();
    assert!(
        constrained
            .import_blob(&mut Cursor::new(b"full"), &d, 4)
            .is_err()
    );
    assert_eq!(std::fs::read(&full_path).unwrap(), b"bad!");
    drop(constrained);
    std::fs::remove_file(&full_path).unwrap();
    std::fs::write(&full_path, b"full").unwrap();
    std::fs::set_permissions(&full_path, std::fs::Permissions::from_mode(0o400)).unwrap();
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
