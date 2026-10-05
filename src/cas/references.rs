use super::Store;
use anyhow::{Result, ensure};
use artifactd_protocol::ArtifactDigest;

impl Store {
    pub fn pin(&mut self, id: &str, digest: &ArtifactDigest) -> Result<()> {
        self.pin_for(id, digest, &crate::state::PeerIdentity::current())
    }

    pub(crate) fn pin_for(
        &mut self,
        id: &str,
        digest: &ArtifactDigest,
        caller: &crate::state::PeerIdentity,
    ) -> Result<()> {
        ensure!(
            !id.is_empty() && id.len() <= 128,
            "invalid reference identity"
        );
        self.open_blob(digest)?;
        self.verify_graph_if_known(digest)?;
        self.db.transaction(|tx| {
            if let Some(reference) = tx.get::<crate::state::Reference>("pins", id)? {
                reference.validate_pin()?;
                reference.authorize(caller)?;
                ensure!(reference.digest == *digest, "reference identity conflict");
            } else {
                ensure!(tx.count("pins")? < 100_000, "reference count limit");
                tx.put(
                    "pins",
                    id,
                    &crate::state::Reference {
                        digest: digest.clone(),
                        owner: Some(caller.clone()),
                        grantee: None,
                        lease_state: None,
                    },
                )?;
            }
            Ok(())
        })
    }

    pub fn unpin(&mut self, id: &str) -> Result<()> {
        self.unpin_for(id, &crate::state::PeerIdentity::current())
    }

    pub(crate) fn unpin_for(
        &mut self,
        id: &str,
        caller: &crate::state::PeerIdentity,
    ) -> Result<()> {
        self.db.transaction(|tx| {
            if let Some(reference) = tx.get::<crate::state::Reference>("pins", id)? {
                reference.validate_pin()?;
                reference.authorize(caller)?;
                tx.remove("pins", id)?;
            }
            Ok(())
        })
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
                    if table == "pins" {
                        reference.validate_pin()?;
                    } else {
                        reference.validate_lease()?;
                    }
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
