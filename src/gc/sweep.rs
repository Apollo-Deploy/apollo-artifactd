use crate::{Store, state};
use anyhow::{Result, ensure};
use artifactd_protocol::ArtifactDigest;

impl Store {
    pub fn gc(&mut self, max: u32) -> Result<u32> {
        ensure!((1..=4096).contains(&max), "invalid gc bound");
        self.reconcile(max)?;
        let snapshot_ready = self.gc_snapshot_ready()?;
        if !snapshot_ready {
            return Ok(0);
        }
        self.gc_prepared(max)?;
        let cursor = self.db.get::<String>("metadata", "blob_gc_cursor")?;
        let page = self
            .db
            .scan::<state::Blob>("blobs", cursor.as_deref(), max as usize)?;
        let next = if page.len() == max as usize {
            page.last().map(|(key, _)| key.clone())
        } else {
            None
        };
        let mut candidates = Vec::new();
        for (key, blob) in page {
            let digest: ArtifactDigest = key.parse()?;
            if !self.gc_marked(&digest, false)? {
                candidates.push((digest, blob.size));
            }
        }
        self.db.transaction(|tx| {
            for (digest, size) in &candidates {
                tx.put("gc", digest.as_str(), &state::Garbage { size: *size })?;
            }
            if let Some(next) = &next {
                tx.put("metadata", "blob_gc_cursor", next)?;
            } else {
                tx.remove("metadata", "blob_gc_cursor")?;
            }
            Ok(())
        })?;
        self.recover_gc(max)?;
        if next.is_none() {
            self.gc_reset()?;
        }
        Ok(candidates.len() as u32)
    }
}
