use super::Store;
use anyhow::{Result, ensure};
use artifactd_protocol::ArtifactDigest;
use std::collections::BTreeSet;

impl Store {
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
}
