use super::Store;
use crate::filesystem;
use anyhow::{Result, ensure};
use artifactd_protocol::ArtifactDigest;
use serde::de::DeserializeOwned;
use sha2::{Digest, Sha256};
use std::io::{Read, Write};

impl Store {
    /// Only verified, synced bytes can become visible under a final identity.
    pub fn import_blob(
        &mut self,
        reader: &mut impl Read,
        digest: &ArtifactDigest,
        size: u64,
    ) -> Result<()> {
        self.import_stream(reader, Some(digest), size).map(|_| ())
    }

    /// Calculates identity for a trusted producer without buffering its bytes.
    pub fn import_calculated(
        &mut self,
        reader: &mut impl Read,
        size: u64,
    ) -> Result<ArtifactDigest> {
        self.import_stream(reader, None, size)
    }

    fn import_stream(
        &mut self,
        reader: &mut impl Read,
        digest: Option<&ArtifactDigest>,
        size: u64,
    ) -> Result<ArtifactDigest> {
        ensure!(size <= self.limits.max_blob, "blob exceeds limit");
        let mut replacing_size = None;
        let mut replacement_peak_extra = 0u64;
        if let Some(digest) = digest
            && let Some(blob) = self
                .db
                .get::<crate::state::Blob>("blobs", digest.as_str())?
        {
            let stored_size = blob.size;
            ensure!(stored_size == size, "duplicate size mismatch");
            if self.verify_file(digest, size).is_ok() {
                let (actual, actual_size) = super::hash(reader, size)?;
                ensure!(
                    actual == *digest && actual_size == size,
                    "duplicate input mismatch"
                );
                return Ok(digest.clone());
            }
            replacing_size = Some(blob.size);
            // The recorded logical size can understate a corrupt final leaf.
            // Admission must cover both that leaf and the staged replacement
            // while the atomic rename is pending.
            let actual_size = match filesystem::read(&self.blobs, digest.hex()) {
                Ok(file) => file.metadata()?.len(),
                Err(error) if is_not_found(&error) => 0,
                Err(error) => return Err(error),
            };
            replacement_peak_extra = actual_size.saturating_sub(blob.size);
            // A DB-recorded object whose bytes fail verification is repaired
            // through the normal private staging and atomic replacement path.
        }

        let objects = self.db.count("blobs")? + self.db.count("imports")?;
        let pending = self.db.count("imports")?;
        let temporary = sum_sizes(
            &self.db,
            "imports",
            |item: &crate::state::Imported| item.size,
            "temporary",
        )?;
        ensure!(
            objects < self.limits.max_objects || replacing_size.is_some(),
            "object count quota exhausted"
        );
        ensure!(
            pending < u64::from(self.limits.max_pending_imports),
            "pending import quota exhausted"
        );
        ensure!(
            temporary
                .checked_add(size)
                .is_some_and(|n| n <= self.limits.max_temp_bytes),
            "temporary byte quota exhausted"
        );
        let blob_bytes = sum_sizes(
            &self.db,
            "blobs",
            |item: &crate::state::Blob| item.size,
            "blob",
        )?;
        let prepared_bytes = sum_sizes(
            &self.db,
            "prepared",
            |item: &crate::state::Prepared| item.size,
            "prepared",
        )?;
        let usage = blob_bytes
            .checked_add(temporary)
            .and_then(|v| v.checked_add(prepared_bytes))
            .ok_or_else(|| anyhow::anyhow!("store usage overflow"))?;
        ensure!(
            usage
                .checked_add(size)
                .and_then(|value| value.checked_add(replacement_peak_extra))
                .is_some_and(|n| n <= self.limits.max_store),
            "store quota exhausted"
        );
        let id = uuid::Uuid::new_v4().to_string();
        let temp = format!("{id}.part");
        self.db.put(
            "imports",
            &id,
            &crate::state::Imported {
                temp: temp.clone(),
                digest: digest.cloned(),
                size,
            },
        )?;
        let result = (|| {
            let actual = self.stage_blob(reader, &temp, size)?;
            if let Some(expected) = digest {
                ensure!(actual == *expected, "input digest mismatch");
            }
            // The resolved identity must be durable before final publication.
            self.db.put(
                "imports",
                &id,
                &crate::state::Imported {
                    temp: temp.clone(),
                    digest: Some(actual.clone()),
                    size,
                },
            )?;
            self.publish_blob(&temp, &actual, size, replacing_size.is_some())?;
            Ok(actual)
        })();
        if result.is_err() {
            // Keep the intent if cleanup fails: restart can retry safely.
            if self.temp.symlink_metadata(&temp).is_ok() {
                filesystem::owned_remove(&self.temp, &temp)?;
            }
            self.db.remove("imports", &id)?;
        } else if let Ok(actual) = &result {
            let actual = actual.clone();
            self.db.transaction(|tx| {
                if tx
                    .get::<crate::state::Blob>("blobs", actual.as_str())?
                    .is_none()
                {
                    tx.put("blobs", actual.as_str(), &crate::state::Blob { size })?;
                }
                tx.remove("imports", &id)
            })?;
        }
        result
    }

    fn stage_blob(
        &self,
        reader: &mut impl Read,
        temp: &str,
        expected: u64,
    ) -> Result<ArtifactDigest> {
        let mut output = filesystem::create(&self.temp, temp)?;
        let mut hash = Sha256::new();
        let mut size = 0u64;
        let mut buffer = [0u8; 64 * 1024];
        loop {
            let n = reader.read(&mut buffer)?;
            if n == 0 {
                break;
            }
            size = size
                .checked_add(n as u64)
                .ok_or_else(|| anyhow::anyhow!("size overflow"))?;
            ensure!(size <= expected, "input exceeds declared size");
            hash.update(&buffer[..n]);
            output.write_all(&buffer[..n])?;
        }
        ensure!(size == expected, "input size mismatch");
        let actual = format!("sha256:{}", hex::encode(hash.finalize())).parse()?;
        filesystem::readonly(&output)?;
        filesystem::sync(&self.temp)?;
        Ok(actual)
    }

    fn publish_blob(
        &self,
        temp: &str,
        digest: &ArtifactDigest,
        expected: u64,
        allow_replacement: bool,
    ) -> Result<()> {
        match self.temp.hard_link(temp, &self.blobs, digest.hex()) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                if self.verify_file(digest, expected).is_ok() {
                    self.temp.remove_file(temp)?;
                    filesystem::sync(&self.temp)?;
                    return Ok(());
                }
                ensure!(
                    allow_replacement,
                    "unrecorded digest collision refuses replacement"
                );
                // Validate the existing leaf's ownership, type, link count and
                // permissions before an atomic same-directory replacement.
                let _existing = filesystem::read(&self.blobs, digest.hex())?;
                self.temp.rename(temp, &self.blobs, digest.hex())?;
                filesystem::sync(&self.temp)?;
                filesystem::sync(&self.blobs)?;
                self.verify_file(digest, expected)?;
                return Ok(());
            }
            Err(e) => return Err(e.into()),
        }
        // Publication can briefly have two links; recovery recognizes this exact
        // intent and inode pair. Readers never run concurrently with mutation.
        self.temp.remove_file(temp)?;
        filesystem::sync(&self.temp)?;
        filesystem::sync(&self.blobs)?;
        self.verify_file(digest, expected)?;
        Ok(())
    }
}

fn sum_sizes<T: DeserializeOwned>(
    db: &crate::state::State,
    table: &'static str,
    size: impl Fn(&T) -> u64,
    label: &str,
) -> Result<u64> {
    let mut total = 0u64;
    let mut after = None;
    loop {
        let page = db.scan::<T>(table, after.as_deref(), 4096)?;
        if page.is_empty() {
            return Ok(total);
        }
        for (_, item) in &page {
            total = total
                .checked_add(size(item))
                .ok_or_else(|| anyhow::anyhow!("{label} bytes overflow"))?;
        }
        after = page.last().map(|(key, _)| key.clone());
    }
}

fn is_not_found(error: &anyhow::Error) -> bool {
    error.downcast_ref::<rustix::io::Errno>() == Some(&rustix::io::Errno::NOENT)
        || error
            .downcast_ref::<std::io::Error>()
            .is_some_and(|error| error.kind() == std::io::ErrorKind::NotFound)
}
