//! Graph admission and optional durable protection publish in one transaction.
use super::{facts, platform_of, select};
use crate::Store;
use anyhow::{Result, ensure};
use artifactd_protocol::{ArtifactDigest, Platform};
use std::collections::BTreeMap;

impl Store {
    pub fn admit_oci(
        &mut self,
        digest: &ArtifactDigest,
        platform: &Platform,
    ) -> Result<serde_json::Value> {
        let caller = crate::state::PeerIdentity::current();
        self.admit_oci_with_pin(digest, platform, None, &caller)
    }

    pub fn admit_oci_pinned(
        &mut self,
        digest: &ArtifactDigest,
        platform: &Platform,
        pin: &str,
    ) -> Result<serde_json::Value> {
        let caller = crate::state::PeerIdentity::current();
        self.admit_oci_with_pin(digest, platform, Some(pin), &caller)
    }

    pub(crate) fn admit_oci_with_pin(
        &mut self,
        digest: &ArtifactDigest,
        platform: &Platform,
        pin: Option<&str>,
        caller: &crate::state::PeerIdentity,
    ) -> Result<serde_json::Value> {
        ensure!(platform.validate(), "unsupported platform");
        let graph = self.graph(digest)?;
        let image = select(&graph, platform)?;
        let mut facts = facts(digest, image, platform);
        if let Some(id) = pin {
            artifactd_protocol::PinId::try_from(id.to_owned()).map_err(anyhow::Error::msg)?;
        }
        self.db.transaction(|tx| {
            if let Some(id) = pin {
                let existing = tx.get::<crate::state::Reference>("pins", id)?;
                if let Some(reference) = existing.as_ref() {
                    reference.authorize(caller)?;
                }
                ensure!(
                    existing.as_ref().is_none_or(|v| v.digest == *digest),
                    "reference identity conflict"
                );
                ensure!(
                    existing.is_some() || tx.count("pins")? < 100_000,
                    "reference count limit"
                );
            }
            let mut by_parent: BTreeMap<&ArtifactDigest, Vec<ArtifactDigest>> = BTreeMap::new();
            for (parent, child) in &graph.edges {
                by_parent.entry(parent).or_default().push(child.clone());
            }
            for (parent, children) in by_parent {
                let mut current = tx
                    .get::<Vec<ArtifactDigest>>("edges", parent.as_str())?
                    .unwrap_or_default();
                for child in children {
                    if !current.contains(&child) {
                        current.push(child);
                    }
                }
                tx.put("edges", parent.as_str(), &current)?;
            }
            for image in graph.images.values() {
                let root = crate::state::Root {
                    kind: "manifest".to_owned(),
                    platform: Some(platform_of(&image.config)),
                };
                if tx
                    .get::<crate::state::Root>("roots", image.digest.as_str())?
                    .is_none()
                {
                    tx.put("roots", image.digest.as_str(), &root)?;
                }
            }
            if tx
                .get::<crate::state::Root>("roots", digest.as_str())?
                .is_none()
            {
                tx.put(
                    "roots",
                    digest.as_str(),
                    &crate::state::Root {
                        kind: "oci".to_owned(),
                        platform: None,
                    },
                )?;
            }
            if let Some(id) = pin {
                tx.put(
                    "pins",
                    id,
                    &crate::state::Reference {
                        digest: digest.clone(),
                        owner: Some(caller.clone()),
                    },
                )?;
            }
            Ok(())
        })?;
        if let Some(id) = pin {
            facts["pin_id"] = serde_json::json!(id);
        }
        Ok(facts)
    }
}
