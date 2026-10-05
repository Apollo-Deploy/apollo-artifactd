use apollo_artifactd::{Limits, Store, state::State};
use artifactd_protocol::{ArtifactDigest, Platform};
use sha2::{Digest, Sha256};
use std::{
    fs::OpenOptions,
    io::{Cursor, Seek, SeekFrom, Write},
    os::unix::fs::PermissionsExt,
};

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
                grantee: None,
                lease_state: None,
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
    let minted_lease = if lease {
        Some(store.lease(&second).unwrap())
    } else {
        None
    };
    if !lease {
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
        store
            .release(minted_lease.as_ref().unwrap().as_str())
            .unwrap();
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

#[test]
fn gc_protects_admitted_oci_graph_without_rehashing_corrupt_live_bytes() {
    let root = root();
    let mut store = Store::open(root.path(), Limits::default()).unwrap();
    let layer = b"corruptible layer bytes";
    let layer_digest = digest(layer);
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
    for (bytes, digest) in [
        (&config[..], &config_digest),
        (&manifest[..], &manifest_digest),
        (&layer[..], &layer_digest),
    ] {
        store
            .import_blob(&mut Cursor::new(bytes), digest, bytes.len() as u64)
            .unwrap();
    }
    let platform = Platform {
        os: "linux".into(),
        architecture: "amd64".into(),
        variant: None,
    };
    store.admit_oci(&manifest_digest, &platform).unwrap();
    store.pin("live-image", &manifest_digest).unwrap();

    let collectible = b"unreferenced content";
    let collectible_digest = digest(collectible);
    store
        .import_blob(
            &mut Cursor::new(collectible),
            &collectible_digest,
            collectible.len() as u64,
        )
        .unwrap();

    // A later read must detect corruption, while graph verification and GC
    // protect a size-matching layer without rehashing it on every GC pass.
    let layer_path = root.path().join("blobs").join(layer_digest.hex());
    std::fs::set_permissions(&layer_path, std::fs::Permissions::from_mode(0o600)).unwrap();
    let mut file = OpenOptions::new().write(true).open(&layer_path).unwrap();
    file.seek(SeekFrom::Start(0)).unwrap();
    file.write_all(b"!").unwrap();
    file.sync_all().unwrap();
    drop(file);
    std::fs::set_permissions(&layer_path, std::fs::Permissions::from_mode(0o400)).unwrap();
    assert!(store.open_blob(&layer_digest).is_err());

    let mut collected = false;
    for _ in 0..128 {
        store.gc(1).unwrap();
        if store.open_blob(&collectible_digest).is_err() {
            collected = true;
            break;
        }
    }
    assert!(collected, "GC did not collect unrelated content");
    assert!(store.open_blob(&manifest_digest).is_ok());
    assert!(
        layer_path.exists(),
        "GC removed corrupted but reachable layer"
    );
}

#[test]
fn gc_resumes_large_oci_proof_before_sweeping_and_restarts_safely() {
    let root = root();
    let mut store = Store::open(root.path(), Limits::default()).unwrap();
    let layer = b"shared layer bytes";
    let layer_digest = digest(layer);
    let mut descriptors = Vec::new();
    let mut manifests = Vec::new();
    let mut configs = Vec::new();
    let padding = "x".repeat(3 << 20);

    for architecture in ["amd64", "arm64"] {
        let config = serde_json::to_vec(&serde_json::json!({
            "architecture": architecture,
            "os": "linux",
            "rootfs": {"type": "layers", "diff_ids": [layer_digest]},
        }))
        .unwrap();
        let config_digest = digest(&config);
        let manifest = serde_json::to_vec(&serde_json::json!({
            "schemaVersion": 2,
            "mediaType": "application/vnd.oci.image.manifest.v1+json",
            "annotations": {"test.padding": padding.clone()},
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
        assert!(manifest.len() as u64 <= Limits::default().max_metadata);
        let manifest_digest = digest(&manifest);
        descriptors.push(serde_json::json!({
            "mediaType": "application/vnd.oci.image.manifest.v1+json",
            "digest": manifest_digest,
            "size": manifest.len(),
            "platform": {"os": "linux", "architecture": architecture},
        }));
        configs.push((config, config_digest));
        manifests.push((manifest, manifest_digest));
    }

    let index = serde_json::to_vec(&serde_json::json!({
        "schemaVersion": 2,
        "mediaType": "application/vnd.oci.image.index.v1+json",
        "manifests": descriptors,
    }))
    .unwrap();
    let index_digest = digest(&index);
    for (bytes, expected) in configs
        .iter()
        .chain(manifests.iter())
        .map(|(bytes, digest)| (bytes.as_slice(), digest))
        .chain([
            (layer.as_slice(), &layer_digest),
            (index.as_slice(), &index_digest),
        ])
    {
        store
            .import_blob(&mut Cursor::new(bytes), expected, bytes.len() as u64)
            .unwrap();
    }
    store
        .admit_oci(
            &index_digest,
            &Platform {
                os: "linux".into(),
                architecture: "amd64".into(),
                variant: None,
            },
        )
        .unwrap();
    store.pin("large-live-index", &index_digest).unwrap();
    let garbage = b"unreferenced garbage";
    let garbage_digest = digest(garbage);
    store
        .import_blob(
            &mut Cursor::new(garbage),
            &garbage_digest,
            garbage.len() as u64,
        )
        .unwrap();

    store.gc(1).unwrap();
    assert!(store.open_blob(&garbage_digest).is_ok());
    drop(store);

    // Restart discards an incomplete mark snapshot. The next bounded pass
    // rebuilds it and still cannot sweep while the OCI proof is partial.
    let mut store = Store::open(root.path(), Limits::default()).unwrap();
    assert!(store.open_blob(&garbage_digest).is_ok());
    let mut collected = false;
    for _ in 0..64 {
        store.gc(1).unwrap();
        if store.open_blob(&garbage_digest).is_err() {
            collected = true;
            break;
        }
        assert!(store.open_blob(&index_digest).is_ok());
    }
    assert!(collected, "bounded GC did not finish the large OCI graph");
    assert!(store.open_blob(&index_digest).is_ok());
    assert!(store.open_blob(&layer_digest).is_ok());
    for (_, manifest_digest) in manifests {
        assert!(store.open_blob(&manifest_digest).is_ok());
    }
    for (_, config_digest) in configs {
        assert!(store.open_blob(&config_digest).is_ok());
    }
}
