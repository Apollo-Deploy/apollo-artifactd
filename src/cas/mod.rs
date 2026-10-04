//! Streaming CAS with durable import intents and exclusive store ownership.
mod import;
mod recovery;
mod references;
use crate::{filesystem, state};
use anyhow::{Result, ensure};
use artifactd_protocol::ArtifactDigest;
use cap_std::fs::Dir;
use rusqlite::Connection;
use sha2::{Digest, Sha256};
use std::{
    fs::File,
    io::{Read, Seek, SeekFrom},
    path::Path,
};

#[derive(Clone, Copy)]
pub struct Limits {
    pub max_blob: u64,
    pub max_store: u64,
    pub max_metadata: u64,
    pub max_graph: usize,
    pub max_output: u64,
    pub max_entry: u64,
    pub max_entries: usize,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            max_blob: 4 << 30,
            max_store: 16 << 30,
            max_metadata: 4 << 20,
            max_graph: 128,
            max_output: 16 << 30,
            max_entry: 4 << 30,
            max_entries: 100_000,
        }
    }
}

pub struct Store {
    pub(crate) root: Dir,
    pub(crate) blobs: Dir,
    pub(crate) temp: Dir,
    pub(crate) prepared: Dir,
    pub(crate) db: Connection,
    pub limits: Limits,
    _owner: File,
}

impl Store {
    pub fn open(path: &Path, limits: Limits) -> Result<Self> {
        let root = filesystem::private_root(path)?;
        let owner = filesystem::lock(&root)?;
        let blobs = filesystem::directory(&root, "blobs")?;
        let temp = filesystem::directory(&root, "temp")?;
        let prepared = filesystem::directory(&root, "prepared")?;
        let db = state::open(&root, path)?;
        let mut store = Self {
            root,
            blobs,
            temp,
            prepared,
            db,
            limits,
            _owner: owner,
        };
        store.reconcile(1024)?;
        Ok(store)
    }

    /// Rehashes bytes before exposing an immutable read handle.
    pub fn open_blob(&self, digest: &ArtifactDigest) -> Result<File> {
        let expected: u64 = self.db.query_row(
            "SELECT size FROM blobs WHERE digest=?1",
            [digest.as_str()],
            |r| crate::state::unsigned(r, 0),
        )?;
        self.verify_file(digest, expected)
    }

    pub(crate) fn verify_file(&self, digest: &ArtifactDigest, expected: u64) -> Result<File> {
        ensure!(expected <= self.limits.max_blob, "blob exceeds limit");
        let mut file = filesystem::read(&self.blobs, digest.hex())?;
        ensure!(file.metadata()?.len() == expected, "blob size mismatch");
        let (actual, size) = hash(&mut file, expected)?;
        ensure!(
            actual == *digest && size == expected,
            "blob integrity mismatch"
        );
        file.seek(SeekFrom::Start(0))?;
        Ok(file)
    }

    pub fn metadata(&self, digest: &ArtifactDigest) -> Result<Vec<u8>> {
        let file = self.open_blob(digest)?;
        ensure!(
            file.metadata()?.len() <= self.limits.max_metadata,
            "metadata exceeds limit"
        );
        let mut bytes = Vec::new();
        file.take(self.limits.max_metadata + 1)
            .read_to_end(&mut bytes)?;
        ensure!(
            bytes.len() as u64 <= self.limits.max_metadata,
            "metadata exceeds limit"
        );
        Ok(bytes)
    }

    pub fn status(&self) -> Result<serde_json::Value> {
        let (count, bytes): (u64, u64) = self.db.query_row(
            "SELECT count(*), coalesce(sum(size),0) FROM blobs",
            [],
            |r| Ok((crate::state::unsigned(r, 0)?, crate::state::unsigned(r, 1)?)),
        )?;
        Ok(
            serde_json::json!({"blobs": count, "bytes": bytes, "schema": 1, "production_qualified": false}),
        )
    }

    pub fn doctor(&self) -> Result<serde_json::Value> {
        let check: String = self.db.query_row("PRAGMA quick_check", [], |r| r.get(0))?;
        ensure!(check == "ok", "state integrity check failed");
        filesystem::sync(&self.root)?;
        Ok(serde_json::json!({"database": "ok", "durability": "FULL", "exclusive_owner": true}))
    }
}

pub(crate) fn hash(reader: &mut impl Read, limit: u64) -> Result<(ArtifactDigest, u64)> {
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
        ensure!(size <= limit, "input exceeds size limit");
        hash.update(&buffer[..n]);
    }
    Ok((
        format!("sha256:{}", hex::encode(hash.finalize())).parse()?,
        size,
    ))
}
