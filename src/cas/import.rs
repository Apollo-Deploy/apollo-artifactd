use super::Store;
use crate::filesystem;
use anyhow::{Result, ensure};
use artifactd_protocol::ArtifactDigest;
use rusqlite::OptionalExtension;
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
        ensure!(size <= self.limits.max_blob, "blob exceeds limit");
        let existing: Option<u64> = self
            .db
            .query_row(
                "SELECT size FROM blobs WHERE digest=?1",
                [digest.as_str()],
                |r| crate::state::unsigned(r, 0),
            )
            .optional()?;
        if let Some(stored_size) = existing {
            ensure!(stored_size == size, "duplicate size mismatch");
            self.verify_file(digest, size)?;
            let (actual, actual_size) = super::hash(reader, size)?;
            ensure!(
                actual == *digest && actual_size == size,
                "duplicate input mismatch"
            );
            return Ok(());
        }

        let usage: u64 = self.db.query_row("SELECT (SELECT coalesce(sum(size),0) FROM blobs)+(SELECT coalesce(sum(size),0) FROM imports)+(SELECT coalesce(sum(size),0) FROM prepared)", [], |r| crate::state::unsigned(r,0))?;
        ensure!(
            usage
                .checked_add(size)
                .is_some_and(|n| n <= self.limits.max_store),
            "store quota exhausted"
        );
        let id = uuid::Uuid::new_v4().to_string();
        let temp = format!("{id}.part");
        self.db.execute(
            "INSERT INTO imports(id,temp,digest,size) VALUES (?1,?2,?3,?4)",
            rusqlite::params![id, temp, digest.as_str(), i64::try_from(size)?],
        )?;
        let result = self.write_blob(reader, &temp, digest, size);
        if result.is_err() {
            // Keep the intent if cleanup fails: restart can retry safely.
            if self.temp.symlink_metadata(&temp).is_ok() {
                filesystem::owned_remove(&self.temp, &temp)?;
            }
            self.db.execute("DELETE FROM imports WHERE id=?1", [&id])?;
        } else {
            let tx = self.db.transaction()?;
            tx.execute(
                "INSERT INTO blobs(digest,size) VALUES (?1,?2) ON CONFLICT(digest) DO NOTHING",
                rusqlite::params![digest.as_str(), i64::try_from(size)?],
            )?;
            tx.execute("DELETE FROM imports WHERE id=?1", [&id])?;
            tx.commit()?;
        }
        result
    }

    fn write_blob(
        &self,
        reader: &mut impl Read,
        temp: &str,
        digest: &ArtifactDigest,
        expected: u64,
    ) -> Result<()> {
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
        ensure!(
            hex::encode(hash.finalize()) == digest.hex(),
            "input digest mismatch"
        );
        filesystem::readonly(&output)?;
        filesystem::sync(&self.temp)?;
        match self.temp.hard_link(temp, &self.blobs, digest.hex()) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                self.verify_file(digest, expected)?;
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
