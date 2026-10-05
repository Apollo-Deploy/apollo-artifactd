use super::{Store, platform_of};
use crate::{
    filesystem,
    state::{Blob, Root},
};
use anyhow::{Result, ensure};
use artifactd_protocol::ArtifactDigest;
use std::collections::BTreeSet;

impl Store {
    /// Reconstructs admitted OCI relationships from verified metadata and
    /// compares them with durable edges. Layer bytes get ownership/size checks
    /// here; their content hashes are checked when read or explicitly verified.
    pub(crate) fn gc_reachable_nodes(
        &self,
        digest: &ArtifactDigest,
    ) -> Result<BTreeSet<ArtifactDigest>> {
        let root = self.db.get::<Root>("roots", digest.as_str())?;
        let blob = self
            .db
            .get::<Blob>("blobs", digest.as_str())?
            .ok_or_else(|| anyhow::anyhow!("missing blob"))?;
        ensure!(
            blob.size <= self.limits.max_blob,
            "reachable blob exceeds limit"
        );
        let looks_oci = if root.is_none() && blob.size <= self.limits.max_metadata {
            let bytes = self.metadata(digest)?;
            serde_json::from_slice::<serde_json::Value>(&bytes).is_ok_and(|value| {
                value.get("schemaVersion").and_then(|v| v.as_u64()) == Some(2)
                    && (value.get("manifests").is_some()
                        || (value.get("config").is_some() && value.get("layers").is_some()))
            })
        } else {
            false
        };

        if root.is_some() || looks_oci {
            return self.gc_oci_reachable_nodes(digest);
        }

        let file = filesystem::read(&self.blobs, digest.hex())?;
        ensure!(
            file.metadata()?.len() == blob.size,
            "reachable blob size mismatch"
        );
        ensure!(
            self.db
                .get::<Vec<ArtifactDigest>>("edges", digest.as_str())?
                .is_none_or(|children| children.is_empty()),
            "raw blob has persisted graph edges"
        );
        Ok(BTreeSet::from([digest.clone()]))
    }

    fn gc_oci_reachable_nodes(&self, digest: &ArtifactDigest) -> Result<BTreeSet<ArtifactDigest>> {
        let graph = self.graph_for_gc(digest)?;
        ensure!(
            graph.edges.len() <= self.limits.max_graph,
            "OCI graph edges exceed bound"
        );
        let mut nodes = graph.nodes.clone();
        for (parent, child) in &graph.edges {
            nodes.insert(parent.clone());
            nodes.insert(child.clone());
        }
        ensure!(
            nodes.len() <= self.limits.max_graph.saturating_add(1),
            "OCI graph nodes exceed bound"
        );

        let mut persisted = BTreeSet::new();
        for parent in &nodes {
            if let Some(children) = self
                .db
                .get::<Vec<ArtifactDigest>>("edges", parent.as_str())?
            {
                for child in children {
                    persisted.insert((parent.clone(), child));
                }
            }
        }
        ensure!(
            persisted.len() <= self.limits.max_graph,
            "persisted graph exceeds bound"
        );
        ensure!(
            persisted == graph.edges,
            "persisted OCI reachability mismatch"
        );

        let root_record = self
            .db
            .get::<Root>("roots", digest.as_str())?
            .ok_or_else(|| anyhow::anyhow!("OCI root classification is missing"))?;
        let expected = if let Some(image) = graph.images.get(digest) {
            Root {
                kind: "manifest".into(),
                platform: Some(platform_of(&image.config)),
            }
        } else {
            Root {
                kind: "oci".into(),
                platform: None,
            }
        };
        ensure!(root_record == expected, "OCI root classification mismatch");
        for image in graph.images.values() {
            let expected = Root {
                kind: "manifest".into(),
                platform: Some(platform_of(&image.config)),
            };
            ensure!(
                self.db
                    .get::<Root>("roots", image.digest.as_str())?
                    .as_ref()
                    == Some(&expected),
                "OCI manifest classification mismatch"
            );
        }
        Ok(nodes)
    }
}
