use apollo_artifactd::Store;
use artifactd_protocol::{ArtifactDigest, Platform};
use sha2::{Digest, Sha256};
use std::io::Cursor;
pub const MANIFEST: &str = "application/vnd.oci.image.manifest.v1+json";

pub fn digest(bytes: &[u8]) -> ArtifactDigest {
    format!("sha256:{}", hex::encode(Sha256::digest(bytes)))
        .parse()
        .unwrap()
}
pub fn import(store: &mut Store, bytes: &[u8]) -> ArtifactDigest {
    let d = digest(bytes);
    store
        .import_blob(&mut Cursor::new(bytes), &d, bytes.len() as u64)
        .unwrap();
    d
}
pub fn platform(arch: &str) -> Platform {
    Platform {
        os: "linux".into(),
        architecture: arch.into(),
        variant: None,
    }
}
pub fn layer(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let mut tar = tar::Builder::new(Vec::new());
    for (path, data) in entries {
        let mut header = tar::Header::new_gnu();
        header.set_size(data.len() as u64);
        header.set_mode(0o600);
        header.set_cksum();
        tar.append_data(&mut header, path, Cursor::new(data))
            .unwrap();
    }
    tar.into_inner().unwrap()
}
pub fn image(
    store: &mut Store,
    arch: &str,
    layers: &[Vec<u8>],
    wrong_diff: bool,
) -> ArtifactDigest {
    let desc: Vec<_> = layers.iter().map(|b| {
        let d = import(store,b);
        serde_json::json!({"mediaType":"application/vnd.oci.image.layer.v1.tar","digest":d,"size":b.len()})
    }).collect();
    let diffs: Vec<_> = layers
        .iter()
        .map(|b| {
            if wrong_diff {
                digest(b"wrong")
            } else {
                digest(b)
            }
        })
        .collect();
    let config = serde_json::to_vec(&serde_json::json!({"architecture":arch,"os":"linux","rootfs":{"type":"layers","diff_ids":diffs}})).unwrap();
    let config_digest = import(store, &config);
    let manifest = serde_json::to_vec(&serde_json::json!({"schemaVersion":2,"mediaType":MANIFEST,"config":{"mediaType":"application/vnd.oci.image.config.v1+json","digest":config_digest,"size":config.len()},"layers":desc})).unwrap();
    let digest = import(store, &manifest);
    store.admit_oci(&digest, &platform(arch)).unwrap();
    digest
}
