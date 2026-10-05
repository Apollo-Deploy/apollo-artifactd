use apollo_artifactd::{Limits, Store};
use artifactd_protocol::{ArtifactDigest, Platform};
use sha2::{Digest, Sha256};
use std::{io::Cursor, os::unix::fs::PermissionsExt};

const MANIFEST: &str = "application/vnd.oci.image.manifest.v1+json";

fn digest(bytes: &[u8]) -> ArtifactDigest {
    format!("sha256:{}", hex::encode(Sha256::digest(bytes)))
        .parse()
        .unwrap()
}

fn platform() -> Platform {
    Platform {
        os: "linux".into(),
        architecture: "amd64".into(),
        variant: None,
    }
}

fn zero_layer_image(
    store: &mut Store,
) -> (
    artifactd_protocol::ArtifactDigest,
    artifactd_protocol::ArtifactDigest,
    Vec<u8>,
) {
    for nonce in 0..10_000u32 {
        let config = serde_json::to_vec(&serde_json::json!({
            "architecture": "amd64",
            "os": "linux",
            "rootfs": {"type": "layers", "diff_ids": []},
            "x-fixture-nonce": nonce,
        }))
        .unwrap();
        let config_digest = digest(&config);
        let manifest = serde_json::to_vec(&serde_json::json!({
            "schemaVersion": 2,
            "mediaType": MANIFEST,
            "config": {
                "mediaType": "application/vnd.oci.image.config.v1+json",
                "digest": config_digest,
                "size": config.len(),
            },
            "layers": [],
        }))
        .unwrap();
        let manifest_digest = digest(&manifest);
        if config_digest < manifest_digest {
            store
                .import_blob(
                    &mut Cursor::new(&config),
                    &config_digest,
                    config.len() as u64,
                )
                .unwrap();
            store
                .import_blob(
                    &mut Cursor::new(&manifest),
                    &manifest_digest,
                    manifest.len() as u64,
                )
                .unwrap();
            return (manifest_digest, config_digest, config);
        }
    }
    panic!("fixture search failed to put config before manifest");
}

#[test]
fn gc_reimport_restores_admitted_manifest_reachability() {
    let root = tempfile::tempdir().unwrap();
    std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let mut store = Store::open(root.path(), Limits::default()).unwrap();
    let (manifest, config, config_bytes) = zero_layer_image(&mut store);
    store.admit_oci(&manifest, &platform()).unwrap();

    assert!(store.open_blob(&manifest).is_ok());
    assert_eq!(store.gc(1).unwrap(), 1);
    assert!(store.open_blob(&config).is_err());
    assert!(store.open_blob(&manifest).is_ok());
    assert!(store.pin("missing-child", &manifest).is_err());
    assert!(store.resolve(&manifest, &platform()).is_err());

    // Reimport the exact bytes from the admitted manifest's descriptor.
    let manifest_json: serde_json::Value =
        serde_json::from_slice(&store.metadata(&manifest).unwrap()).unwrap();
    assert_eq!(manifest_json["config"]["digest"], serde_json::json!(config));
    store
        .import_blob(
            &mut Cursor::new(&config_bytes),
            &config,
            config_bytes.len() as u64,
        )
        .unwrap();

    drop(store);
    let mut store = Store::open(root.path(), Limits::default()).unwrap();
    store.pin("restored", &manifest).unwrap();
    assert_eq!(store.gc(1).unwrap(), 0);
    assert!(store.open_blob(&config).is_ok());
    assert!(store.open_blob(&manifest).is_ok());
}
