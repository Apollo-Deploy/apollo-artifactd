use super::digest;
use anyhow::Result;
use artifactd_protocol::ArtifactDigest;
use std::io::Cursor;

pub fn layer(index: u64) -> Result<Vec<u8>> {
    let mut archive = tar::Builder::new(Vec::new());
    let payload = format!("payload-{index}");
    let mut header = tar::Header::new_gnu();
    header.set_size(payload.len() as u64);
    header.set_mode(0o644);
    header.set_cksum();
    archive.append_data(&mut header, "payload", Cursor::new(payload.into_bytes()))?;
    archive.finish()?;
    Ok(archive.into_inner()?)
}
pub fn archive(index: u64) -> Result<(Vec<u8>, ArtifactDigest)> {
    let layer = layer(index)?;
    let layer_digest = digest(&layer);
    let architecture = if cfg!(target_arch = "aarch64") {
        "arm64"
    } else {
        "amd64"
    };
    let config = serde_json::to_vec(&serde_json::json!({
        "architecture":architecture, "os":"linux",
        "rootfs":{"type":"layers","diff_ids":[layer_digest]}
    }))?;
    let config_digest = digest(&config);
    let manifest = serde_json::to_vec(&serde_json::json!({
        "schemaVersion":2,
        "mediaType":"application/vnd.oci.image.manifest.v1+json",
        "config":{"mediaType":"application/vnd.oci.image.config.v1+json","digest":config_digest,"size":config.len()},
        "layers":[{"mediaType":"application/vnd.oci.image.layer.v1.tar","digest":layer_digest,"size":layer.len()}]
    }))?;
    let manifest_digest = digest(&manifest);
    let index_json = serde_json::to_vec(&serde_json::json!({
        "schemaVersion":2,
        "manifests":[{"mediaType":"application/vnd.oci.image.manifest.v1+json","digest":manifest_digest,"size":manifest.len(),"platform":{"os":"linux","architecture":architecture}}]
    }))?;
    let mut archive = tar::Builder::new(Vec::new());
    for (name, bytes) in [
        (
            "oci-layout".to_owned(),
            br#"{"imageLayoutVersion":"1.0.0"}"#.to_vec(),
        ),
        ("index.json".to_owned(), index_json),
        (format!("blobs/sha256/{}", config_digest.hex()), config),
        (format!("blobs/sha256/{}", manifest_digest.hex()), manifest),
        (format!("blobs/sha256/{}", layer_digest.hex()), layer),
    ] {
        let mut header = tar::Header::new_gnu();
        header.set_size(bytes.len() as u64);
        header.set_mode(0o644);
        header.set_cksum();
        archive.append_data(&mut header, name, Cursor::new(bytes))?;
    }
    Ok((archive.into_inner()?, manifest_digest))
}
