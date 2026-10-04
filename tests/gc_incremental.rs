use apollo_artifactd::{Limits, Store, state::State};
use artifactd_protocol::ArtifactDigest;
use sha2::{Digest, Sha256};
use std::{io::Cursor, os::unix::fs::PermissionsExt};

fn digest(bytes: &[u8]) -> ArtifactDigest {
    format!("sha256:{}", hex::encode(Sha256::digest(bytes)))
        .parse()
        .unwrap()
}

fn root() -> tempfile::TempDir {
    let root = tempfile::tempdir().unwrap();
    std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    root
}

#[test]
fn late_corrupt_pin_is_found_by_bounded_gc_not_startup() {
    let root = root();
    let mut store = Store::open(root.path(), Limits::default()).unwrap();
    let retained = b"retained pinned bytes";
    let collectible = b"collectible bytes";
    let retained_digest = digest(retained);
    let collectible_digest = digest(collectible);
    store
        .import_blob(
            &mut Cursor::new(retained),
            &retained_digest,
            retained.len() as u64,
        )
        .unwrap();
    store
        .import_blob(
            &mut Cursor::new(collectible),
            &collectible_digest,
            collectible.len() as u64,
        )
        .unwrap();
    for index in 0..1025 {
        store
            .pin(&format!("pin-{index:04}"), &retained_digest)
            .unwrap();
    }
    drop(store);

    let dir =
        cap_std::fs::Dir::open_ambient_dir(root.path(), cap_std::ambient_authority()).unwrap();
    let state = State::open(&dir).unwrap();
    let missing: ArtifactDigest = format!("sha256:{}", "f".repeat(64)).parse().unwrap();
    state
        .put(
            "pins",
            "pin-z-late-corrupt",
            &apollo_artifactd::state::Reference {
                digest: missing,
                owner: Some(apollo_artifactd::state::PeerIdentity::current()),
            },
        )
        .unwrap();
    drop(state);

    let mut store = Store::open(root.path(), Limits::default()).unwrap();
    assert!(store.open_blob(&retained_digest).is_ok());
    assert!(store.open_blob(&collectible_digest).is_ok());

    let mut found_late_corruption = false;
    for _ in 0..1100 {
        if store.gc(1).is_err() {
            found_late_corruption = true;
            break;
        }
    }
    assert!(
        found_late_corruption,
        "GC never reached the late corrupt root"
    );
    assert!(store.open_blob(&retained_digest).is_ok());
    assert!(store.open_blob(&collectible_digest).is_ok());
}

fn run_handle_invalidation(root_count: usize, lease: bool) {
    let root = root();
    let mut store = Store::open(root.path(), Limits::default()).unwrap();
    let mut first_bytes = b"gc anchor one".to_vec();
    let mut second_bytes = b"gc target two".to_vec();
    let mut first = digest(&first_bytes);
    let mut second = digest(&second_bytes);
    if second < first {
        std::mem::swap(&mut first_bytes, &mut second_bytes);
        std::mem::swap(&mut first, &mut second);
    }
    for (bytes, digest) in [(&first_bytes, &first), (&second_bytes, &second)] {
        store
            .import_blob(&mut Cursor::new(bytes), digest, bytes.len() as u64)
            .unwrap();
    }

    // The first bounded call can leave a ready mark snapshot or a partial
    // root walk. The new handle must invalidate either state before sweep.
    for index in 0..root_count {
        store.pin(&format!("anchor-{index:02}"), &first).unwrap();
    }
    assert!(store.gc(1).is_ok());
    if lease {
        store.lease("late-target", &second).unwrap();
    } else {
        store.pin("late-target", &second).unwrap();
    }
    // Exercise every bounded call: an early break would only prove that the
    // blobs existed before sweeping reached the newly protected target.
    for _ in 0..128 {
        assert!(store.gc(1).is_ok());
        assert!(store.open_blob(&first).is_ok());
        assert!(store.open_blob(&second).is_ok());
    }
    assert!(store.open_blob(&first).is_ok());
    assert!(store.open_blob(&second).is_ok());

    if lease {
        store.release("late-target").unwrap();
    } else {
        store.unpin("late-target").unwrap();
    }
    let mut collected = false;
    for _ in 0..128 {
        assert!(store.gc(1).is_ok());
        if store.open_blob(&second).is_err() {
            collected = true;
            break;
        }
    }
    assert!(collected, "released target never became collectible");
    assert!(store.open_blob(&first).is_ok());
}

#[test]
fn pin_and_lease_invalidate_ready_and_partial_gc_snapshots() {
    for root_count in [1, 70] {
        for lease in [false, true] {
            run_handle_invalidation(root_count, lease);
        }
    }
}
