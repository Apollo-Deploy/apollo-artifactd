use artifactd_protocol::ArtifactDigest;
use sha2::{Digest, Sha256};
use std::{
    fs::File,
    io::{Cursor, Seek, SeekFrom},
    os::unix::fs::PermissionsExt,
    path::Path,
};

fn digest(bytes: &[u8]) -> ArtifactDigest {
    format!("sha256:{}", hex::encode(Sha256::digest(bytes)))
        .parse()
        .unwrap()
}

pub fn archive() -> (tempfile::NamedTempFile, ArtifactDigest) {
    let layer = {
        let mut tar = tar::Builder::new(Vec::new());
        let data = b"artifactd api registry";
        let mut header = tar::Header::new_gnu();
        header.set_size(data.len() as u64);
        header.set_mode(0o600);
        header.set_cksum();
        tar.append_data(&mut header, "content", Cursor::new(data))
            .unwrap();
        tar.into_inner().unwrap()
    };
    let layer_digest = digest(&layer);
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
    let index = serde_json::to_vec(&serde_json::json!({
        "schemaVersion": 2,
        "mediaType": "application/vnd.oci.image.index.v1+json",
        "manifests": [{
            "mediaType": "application/vnd.oci.image.manifest.v1+json",
            "digest": manifest_digest,
            "size": manifest.len(),
            "platform": {"os": "linux", "architecture": "amd64"},
        }],
    }))
    .unwrap();
    let index_digest = digest(&index);
    let mut archive = tar::Builder::new(Vec::new());
    let mut append = |name: &str, bytes: &[u8]| {
        let mut header = tar::Header::new_gnu();
        header.set_size(bytes.len() as u64);
        header.set_mode(0o600);
        header.set_cksum();
        archive
            .append_data(&mut header, name, Cursor::new(bytes))
            .unwrap();
    };
    append("oci-layout", br#"{"imageLayoutVersion":"1.0.0"}"#);
    append("index.json", &index);
    append(&format!("blobs/sha256/{}", layer_digest.hex()), &layer);
    append(&format!("blobs/sha256/{}", config_digest.hex()), &config);
    append(
        &format!("blobs/sha256/{}", manifest_digest.hex()),
        &manifest,
    );
    let mut file = tempfile::NamedTempFile::new().unwrap();
    std::io::Write::write_all(file.as_file_mut(), &archive.into_inner().unwrap()).unwrap();
    file.as_file_mut().seek(SeekFrom::Start(0)).unwrap();
    (file, index_digest)
}

pub fn private_credentials(path: &Path) -> File {
    let file = File::open(path).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).unwrap();
    file
}

pub fn pinned(reference: &str, digest: &ArtifactDigest) -> String {
    if let Some(at) = reference.rfind('@') {
        return format!("{}@{}", &reference[..at], digest);
    }
    let colon = reference.rfind(':').filter(|index| {
        reference[*index + 1..].chars().all(|c| c != '/')
            && reference[..*index]
                .rfind('/')
                .is_none_or(|slash| slash < *index)
    });
    match colon {
        Some(index) => format!("{}@{}", &reference[..index], digest),
        None => format!("{}@{}", reference, digest),
    }
}
