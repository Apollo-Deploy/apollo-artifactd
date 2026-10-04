use super::Store;
use anyhow::{Result, ensure};
use artifactd_protocol::ArtifactDigest;
use std::collections::BTreeSet;

impl Store {
    pub(crate) fn referenced_by_handles(&self, digest: &ArtifactDigest) -> Result<bool> {
        for table in ["pins", "leases"] {
            let mut after = None;
            loop {
                let page =
                    self.db
                        .scan::<crate::state::Reference>(table, after.as_deref(), 4096)?;
                if page.is_empty() {
                    break;
                }
                for (_, reference) in &page {
                    if self.reachable(&reference.digest, digest)? {
                        return Ok(true);
                    }
                }
                after = page.last().map(|(key, _)| key.clone());
            }
        }
        Ok(false)
    }

    pub fn pin(&mut self, id: &str, digest: &ArtifactDigest) -> Result<()> {
        self.reference("pins", id, digest)
    }
    pub fn lease(&mut self, id: &str, digest: &ArtifactDigest) -> Result<()> {
        self.reference("leases", id, digest)
    }

    fn reference(&mut self, table: &'static str, id: &str, digest: &ArtifactDigest) -> Result<()> {
        ensure!(
            id.len() <= 128 && !id.is_empty(),
            "invalid reference identity"
        );
        ensure!(self.db.count(table)? < 100_000, "reference count limit");
        self.open_blob(digest)?;
        self.verify_graph_if_known(digest)?;
        let existing = self.db.get::<crate::state::Reference>(table, id)?;
        ensure!(
            existing.as_ref().is_none_or(|v| v.digest == *digest),
            "reference identity conflict"
        );
        if existing.is_none() {
            self.db.put(
                table,
                id,
                &crate::state::Reference {
                    digest: digest.clone(),
                },
            )?;
        }
        Ok(())
    }

    pub fn unpin(&mut self, id: &str) -> Result<()> {
        self.db.remove("pins", id)
    }
    pub fn release(&mut self, id: &str) -> Result<()> {
        self.db.remove("leases", id)
    }

    pub fn leased(&self, id: &str, digest: &ArtifactDigest) -> Result<bool> {
        let Some(reference) = self.db.get::<crate::state::Reference>("leases", id)? else {
            return Ok(false);
        };
        self.reachable(&reference.digest, digest)
    }

    pub(crate) fn protected(&self, digest: &ArtifactDigest) -> Result<bool> {
        for table in ["pins", "leases"] {
            let mut after = None;
            loop {
                let page =
                    self.db
                        .scan::<crate::state::Reference>(table, after.as_deref(), 4096)?;
                if page.is_empty() {
                    break;
                }
                for (_, reference) in &page {
                    if self.reachable(&reference.digest, digest)? {
                        return Ok(true);
                    }
                }
                after = page.last().map(|(key, _)| key.clone());
            }
        }
        let mut after = None;
        loop {
            let page =
                self.db
                    .scan::<crate::state::Prepared>("prepared", after.as_deref(), 4096)?;
            if page.is_empty() {
                break;
            }
            for (_, prepared) in &page {
                if prepared.phase == "complete" && self.reachable(&prepared.manifest, digest)? {
                    return Ok(true);
                }
            }
            after = page.last().map(|(key, _)| key.clone());
        }
        Ok(false)
    }

    fn reachable(&self, root: &ArtifactDigest, wanted: &ArtifactDigest) -> Result<bool> {
        let mut seen = BTreeSet::new();
        let mut pending = vec![root.clone()];
        while let Some(current) = pending.pop() {
            if current == *wanted {
                return Ok(true);
            }
            if !seen.insert(current.clone()) {
                continue;
            }
            ensure!(
                seen.len() <= self.limits.max_graph,
                "reachability graph exceeds bound"
            );
            if let Some(children) = self
                .db
                .get::<Vec<ArtifactDigest>>("edges", current.as_str())?
            {
                pending.extend(children);
            }
        }
        Ok(false)
    }

    pub(crate) fn verify_live_graphs(&self) -> Result<()> {
        for table in ["pins", "leases"] {
            let mut after = None;
            loop {
                let page =
                    self.db
                        .scan::<crate::state::Reference>(table, after.as_deref(), 4096)?;
                if page.is_empty() {
                    break;
                }
                for (_, reference) in &page {
                    self.open_blob(&reference.digest)?;
                    self.verify_graph_if_known(&reference.digest)?;
                }
                after = page.last().map(|(key, _)| key.clone());
            }
        }
        let mut after = None;
        loop {
            let page =
                self.db
                    .scan::<crate::state::Prepared>("prepared", after.as_deref(), 4096)?;
            if page.is_empty() {
                break;
            }
            for (_, prepared) in &page {
                if prepared.phase == "complete" {
                    self.open_blob(&prepared.manifest)?;
                    self.verify_graph_if_known(&prepared.manifest)?;
                }
            }
            after = page.last().map(|(key, _)| key.clone());
        }
        Ok(())
    }

    pub fn gc(&mut self, max: u32) -> Result<u32> {
        ensure!(max > 0 && max <= 4096, "invalid gc bound");
        self.verify_live_graphs()?;
        self.reconcile(max)?;
        self.gc_prepared(max)?;
        let mut candidates = Vec::new();
        let cursor = self.db.get::<String>("metadata", "blob_gc_cursor")?;
        let page = self
            .db
            .scan::<crate::state::Blob>("blobs", cursor.as_deref(), max as usize)?;
        let next = if page.len() == max as usize {
            page.last().map(|(key, _)| key.clone())
        } else {
            None
        };
        for (digest, blob) in page {
            let digest: ArtifactDigest = digest.parse()?;
            if !self.protected(&digest)? {
                candidates.push((digest, blob.size));
            }
        }
        // Cursor and deletion intents commit together. A crash before effects
        // leaves all selected candidates recoverable; pinned prefixes cannot
        // permanently starve later blobs, including across restarts.
        self.db.transaction(|tx| {
            for (digest, size) in &candidates {
                tx.put(
                    "gc",
                    digest.as_str(),
                    &crate::state::Garbage { size: *size },
                )?;
            }
            if let Some(next) = next {
                tx.put("metadata", "blob_gc_cursor", &next)?;
            } else {
                tx.remove("metadata", "blob_gc_cursor")?;
            }
            Ok(())
        })?;
        self.recover_gc(max)?;
        Ok(candidates.len() as u32)
    }
}
