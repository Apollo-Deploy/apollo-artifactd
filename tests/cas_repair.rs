use apollo_artifactd::{Limits, Store};
use artifactd_protocol::ArtifactDigest;
use sha2::{Digest, Sha256};
use std::{
    io::{Cursor, Read},
    os::unix::fs::PermissionsExt,
};

fn digest(bytes: &[u8]) -> ArtifactDigest {
    format!("sha256:{}", hex::encode(Sha256::digest(bytes)))
        .parse()
        .unwrap()
}

fn store(max_store: u64) -> (tempfile::TempDir, Store) {
    let root = tempfile::tempdir().unwrap();
    std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let limits = Limits {
        max_store,
        ..Limits::default()
    };
    let store = Store::open(root.path(), limits).unwrap();
    (root, store)
}

#[test]
fn oversized_corrupt_final_counts_toward_repair_peak_quota() {
    // Contract: repair admission accounts for the corrupt final inode while
    // staging its replacement, preserving bytes when that peak exceeds quota.
    // The regression is the old DB-logical-size-only calculation.
    let (root, mut store) = store(160);
    let data = vec![b'a'; 80];
    let digest = digest(&data);
    store
        .import_blob(&mut Cursor::new(&data), &digest, data.len() as u64)
        .unwrap();
    let path = root.path().join("blobs").join(digest.hex());
    std::fs::remove_file(&path).unwrap();
    std::fs::write(&path, vec![b'c'; 200]).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o400)).unwrap();

    let error = store
        .import_blob(&mut Cursor::new(&data), &digest, data.len() as u64)
        .unwrap_err();
    assert!(error.to_string().contains("store quota exhausted"));
    assert_eq!(std::fs::read(&path).unwrap(), vec![b'c'; 200]);
    assert!(store.open_blob(&digest).is_err());

    store.limits.max_store = 320;
    store
        .import_blob(&mut Cursor::new(&data), &digest, data.len() as u64)
        .unwrap();
    let mut output = Vec::new();
    store
        .open_blob(&digest)
        .unwrap()
        .read_to_end(&mut output)
        .unwrap();
    assert_eq!(output, data);
}

#[test]
fn missing_recorded_final_is_repaired_without_new_object() {
    let (root, mut store) = store(160);
    let data = b"missing final";
    let digest = digest(data);
    store
        .import_blob(&mut Cursor::new(data), &digest, data.len() as u64)
        .unwrap();
    std::fs::remove_file(root.path().join("blobs").join(digest.hex())).unwrap();
    store
        .import_blob(&mut Cursor::new(data), &digest, data.len() as u64)
        .unwrap();
    let mut output = Vec::new();
    store
        .open_blob(&digest)
        .unwrap()
        .read_to_end(&mut output)
        .unwrap();
    assert_eq!(output, data);
}
