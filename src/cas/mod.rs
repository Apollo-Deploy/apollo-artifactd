//! Streaming CAS with durable import intents and exclusive store ownership.
mod import;
mod recovery;
mod references;
use crate::{filesystem, state};
use anyhow::{Result, ensure};
use artifactd_protocol::ArtifactDigest;
use cap_std::fs::Dir;
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
    pub max_objects: u64,
    pub max_temp_bytes: u64,
    pub max_pending_imports: u32,
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
            max_objects: 1_000_000,
            max_temp_bytes: 4 << 30,
            max_pending_imports: 4096,
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
    pub(crate) db: state::State,
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
        // A legacy SQLite file is never opened or replaced implicitly. The redb
        // adapter creates only state.redb; callers must perform an explicit
        // migration before reusing a store containing state.sqlite.
        ensure!(
            !path.join("state.sqlite").exists(),
            "legacy SQLite state requires explicit migration"
        );
        let db = state::State::open(&root)?;
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
        let expected = self
            .db
            .get::<state::Blob>("blobs", digest.as_str())?
            .ok_or_else(|| anyhow::anyhow!("blob is not admitted"))?
            .size;
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
        let count = self.db.count("blobs")?;
        let mut bytes = 0u64;
        let mut after = None;
        let mut seen = 0u64;
        loop {
            let entries = self
                .db
                .scan::<state::Blob>("blobs", after.as_deref(), 4096)?;
            if entries.is_empty() {
                break;
            }
            for (_, blob) in &entries {
                bytes = bytes
                    .checked_add(blob.size)
                    .ok_or_else(|| anyhow::anyhow!("blob bytes overflow"))?;
            }
            seen += entries.len() as u64;
            after = entries.last().map(|(key, _)| key.clone());
        }
        ensure!(seen == count, "status scan changed during inspection");
        Ok(
            serde_json::json!({"blobs": count, "bytes": bytes, "schema": crate::state::CURRENT_SCHEMA, "production_qualified": false}),
        )
    }

    pub fn doctor(&mut self) -> Result<serde_json::Value> {
        self.verify_live_graphs()?;
        filesystem::sync(&self.root)?;
        Ok(
            serde_json::json!({"database": "open", "database_integrity_checked": false, "live_graphs_verified":true, "engine": "redb", "durability": "Immediate", "exclusive_owner": true}),
        )
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
