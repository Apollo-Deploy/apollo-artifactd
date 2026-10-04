use super::Store;
use anyhow::{Result, ensure};
use artifactd_protocol::ArtifactDigest;
use std::collections::BTreeSet;

impl Store {
    pub fn pin(&mut self, id: &str, digest: &ArtifactDigest) -> Result<()> {
        self.pin_for(id, digest, &crate::state::PeerIdentity::current())
    }
    pub fn lease(&mut self, id: &str, digest: &ArtifactDigest) -> Result<()> {
        self.lease_for(id, digest, &crate::state::PeerIdentity::current())
    }
    pub(crate) fn pin_for(
        &mut self,
        id: &str,
        digest: &ArtifactDigest,
        caller: &crate::state::PeerIdentity,
    ) -> Result<()> {
        self.reference("pins", id, digest, caller)
    }
    pub(crate) fn lease_for(
        &mut self,
        id: &str,
        digest: &ArtifactDigest,
        caller: &crate::state::PeerIdentity,
    ) -> Result<()> {
        self.reference("leases", id, digest, caller)
    }

    fn reference(
        &mut self,
        table: &'static str,
        id: &str,
        digest: &ArtifactDigest,
        caller: &crate::state::PeerIdentity,
    ) -> Result<()> {
        ensure!(
            id.len() <= 128 && !id.is_empty(),
            "invalid reference identity"
        );
        self.open_blob(digest)?;
        self.verify_graph_if_known(digest)?;
        self.db.transaction(|tx| {
            let existing = tx.get::<crate::state::Reference>(table, id)?;
            if let Some(reference) = &existing {
                reference.authorize(caller)?;
                ensure!(reference.digest == *digest, "reference identity conflict");
            } else {
                ensure!(tx.count(table)? < 100_000, "reference count limit");
                tx.put(
                    table,
                    id,
                    &crate::state::Reference {
                        digest: digest.clone(),
                        owner: Some(caller.clone()),
                    },
                )?;
            }
            Ok(())
        })
    }

    pub fn unpin(&mut self, id: &str) -> Result<()> {
        self.unpin_for(id, &crate::state::PeerIdentity::current())
    }
    pub fn release(&mut self, id: &str) -> Result<()> {
        self.release_for(id, &crate::state::PeerIdentity::current())
    }
    pub(crate) fn unpin_for(
        &mut self,
        id: &str,
        caller: &crate::state::PeerIdentity,
    ) -> Result<()> {
        self.remove_reference("pins", id, caller)
    }
    pub(crate) fn release_for(
        &mut self,
        id: &str,
        caller: &crate::state::PeerIdentity,
    ) -> Result<()> {
        self.remove_reference("leases", id, caller)
    }
    fn remove_reference(
        &self,
        table: &'static str,
        id: &str,
        caller: &crate::state::PeerIdentity,
    ) -> Result<()> {
        self.db.transaction(|tx| {
            if let Some(reference) = tx.get::<crate::state::Reference>(table, id)? {
                reference.authorize(caller)?;
                tx.remove(table, id)?;
            }
            Ok(())
        })
    }

    pub fn leased(&self, id: &str, digest: &ArtifactDigest) -> Result<bool> {
        self.leased_for(id, digest, &crate::state::PeerIdentity::current())
    }
    pub(crate) fn leased_for(
        &self,
        id: &str,
        digest: &ArtifactDigest,
        caller: &crate::state::PeerIdentity,
    ) -> Result<bool> {
        let Some(reference) = self.db.get::<crate::state::Reference>("leases", id)? else {
            return Ok(false);
        };
        reference.authorize(caller)?;
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
                    reference.require_owner()?;
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
