use apollo_artifactd::{Limits, Store};
use artifactd_protocol::ArtifactDigest;
use sha2::{Digest, Sha256};
use std::{io::Cursor, os::unix::fs::PermissionsExt};

fn digest(bytes: &[u8]) -> ArtifactDigest {
    format!("sha256:{}", hex::encode(Sha256::digest(bytes)))
        .parse()
        .unwrap()
}

#[test]
fn local_lease_issuer_never_reuses_id_after_release_and_restart() {
    let root = tempfile::tempdir().unwrap();
    std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let bytes = b"local lease issuer payload";
    let expected = digest(bytes);
    let first = {
        let mut store = Store::open(root.path(), Limits::default()).unwrap();
        store
            .import_blob(&mut Cursor::new(bytes), &expected, bytes.len() as u64)
            .unwrap();
        let lease = store.lease(&expected).unwrap();
        store.release(lease.as_str()).unwrap();
        lease
    };
    let second = {
        let mut store = Store::open(root.path(), Limits::default()).unwrap();
        let lease = store.lease(&expected).unwrap();
        store.release(lease.as_str()).unwrap();
        lease
    };
    assert_ne!(first, second, "released local lease identity was reused");
}

#[test]
fn local_lease_issuer_rejects_partial_persisted_metadata() {
    for corrupt_sequence in [None, Some(0u64)] {
        let root = tempfile::tempdir().unwrap();
        std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        let bytes = b"partial lease issuer payload";
        let expected = digest(bytes);
        {
            let mut store = Store::open(root.path(), Limits::default()).unwrap();
            store
                .import_blob(&mut Cursor::new(bytes), &expected, bytes.len() as u64)
                .unwrap();
            let lease = store.lease(&expected).unwrap();
            store.release(lease.as_str()).unwrap();
        }
        let dir =
            cap_std::fs::Dir::open_ambient_dir(root.path(), cap_std::ambient_authority()).unwrap();
        let state = apollo_artifactd::state::State::open(&dir).unwrap();
        match corrupt_sequence {
            None => state.remove("metadata", "local_lease_sequence").unwrap(),
            Some(value) => state
                .put("metadata", "local_lease_sequence", &value)
                .unwrap(),
        }
        drop(state);

        let mut store = Store::open(root.path(), Limits::default()).unwrap();
        let error = store.lease(&expected).unwrap_err();
        assert!(error.to_string().contains("partial local lease metadata"));
        assert!(store.open_blob(&expected).is_ok());
    }
}
