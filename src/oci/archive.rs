use crate::Store;
use anyhow::{Result, bail, ensure};
use artifactd_protocol::{ArtifactDigest, Platform};
use sha2::{Digest, Sha256};
use std::{collections::BTreeSet, io::Read};

impl Store {
    /// Layout entries are interpreted, never unpacked into arbitrary paths.
    pub fn import_oci_archive(
        &mut self,
        reader: impl Read,
        platform: &Platform,
    ) -> Result<serde_json::Value> {
        let mut archive = tar::Archive::new(reader.take(self.limits.max_store + 1));
        archive.set_extension_size_limit(64 * 1024);
        archive.set_extension_total_limit(64 * 1024 * 1024);
        let mut names = BTreeSet::new();
        let mut index = None;
        let mut layout = false;
        let mut total = 0u64;
        for entry in archive.entries()? {
            let mut entry = entry?;
            crate::archive_policy::validate_pax(&mut entry)?;
            let path = crate::archive_policy::safe_path(entry.path_bytes().as_ref())?;
            let name = path
                .to_str()
                .ok_or_else(|| anyhow::anyhow!("invalid archive path"))?
                .to_owned();
            ensure!(
                names.len() < self.limits.max_entries && names.insert(name.clone()),
                "archive duplicate/entry limit"
            );
            if entry.header().entry_type().is_dir() {
                ensure!(
                    (name.is_empty() && entry.size() == 0)
                        || (matches!(name.as_str(), "blobs" | "blobs/sha256") && entry.size() == 0),
                    "unexpected archive directory"
                );
                continue;
            }
            ensure!(
                entry.header().entry_type().is_file(),
                "archive links/devices/special entries rejected"
            );
            let size = entry.size();
            total = total
                .checked_add(size)
                .ok_or_else(|| anyhow::anyhow!("archive size overflow"))?;
            ensure!(
                total <= self.limits.max_store && size <= self.limits.max_blob,
                "archive exceeds budget"
            );
            if name == "index.json" || name == "oci-layout" {
                ensure!(
                    size <= self.limits.max_metadata,
                    "archive metadata too large"
                );
                let mut bytes = Vec::new();
                entry.read_to_end(&mut bytes)?;
                ensure!(bytes.len() as u64 == size, "truncated archive metadata");
                if name == "oci-layout" {
                    let value: serde_json::Value = serde_json::from_slice(&bytes)?;
                    ensure!(
                        value.get("imageLayoutVersion").and_then(|v| v.as_str()) == Some("1.0.0"),
                        "invalid OCI layout version"
                    );
                    layout = true;
                } else {
                    index = Some(bytes);
                }
            } else if let Some(hex) = name.strip_prefix("blobs/sha256/") {
                let digest: ArtifactDigest = format!("sha256:{hex}").parse()?;
                self.import_blob(&mut entry, &digest, size)?;
            } else {
                bail!("unexpected OCI archive member");
            }
        }
        let mut reader = archive.into_inner();
        let mut tail = [0u8; 65536];
        let mut zeros = 0u64;
        loop {
            let n = reader.read(&mut tail)?;
            if n == 0 {
                break;
            }
            ensure!(
                tail[..n].iter().all(|b| *b == 0),
                "nonzero data after tar terminator"
            );
            zeros += n as u64;
        }
        ensure!(
            zeros >= 512 && reader.limit() > 0,
            "incomplete/oversized OCI archive"
        );
        ensure!(layout, "missing OCI layout marker");
        let bytes = index.ok_or_else(|| anyhow::anyhow!("missing OCI index"))?;
        let digest: ArtifactDigest =
            format!("sha256:{}", hex::encode(Sha256::digest(&bytes))).parse()?;
        self.import_blob(&mut bytes.as_slice(), &digest, bytes.len() as u64)?;
        let mut facts = self.admit_oci(&digest, platform)?;
        facts["artifact_digest"] = serde_json::to_value(&digest)?;
        Ok(facts)
    }
}
