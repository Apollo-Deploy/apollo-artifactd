use super::{Budget, GcWalk, GraphKind, Task, WalkPhase};
use crate::{Store, filesystem, oci, state};
use anyhow::{Result, ensure};
use artifactd_protocol::ArtifactDigest;
use std::collections::BTreeSet;

impl Store {
    pub(in crate::gc) fn gc_advance_walk(
        &self,
        walk: &mut GcWalk,
        budget: &mut Budget,
    ) -> Result<bool> {
        loop {
            match walk.phase {
                WalkPhase::Discover => {
                    let Some(task) = walk.pending.pop() else {
                        walk.phase = WalkPhase::Edges;
                        walk.cursor = 0;
                        continue;
                    };
                    if !self.gc_process_task(walk, task.clone(), budget)? {
                        walk.pending.push(task);
                        return Ok(false);
                    }
                }
                WalkPhase::Edges => {
                    if walk.cursor >= walk.catalog.len() {
                        walk.phase = WalkPhase::Images;
                        walk.cursor = 0;
                        continue;
                    }
                    if !budget.item() {
                        return Ok(false);
                    }
                    self.gc_verify_outgoing(walk, walk.cursor)?;
                    walk.cursor += 1;
                }
                WalkPhase::Images => {
                    if walk.cursor < walk.images.len() {
                        if !budget.item() {
                            return Ok(false);
                        }
                        self.gc_verify_image(walk, walk.cursor)?;
                        walk.cursor += 1;
                        continue;
                    }
                    if !budget.item() {
                        return Ok(false);
                    }
                    self.gc_verify_root_classification(walk)?;
                    walk.phase = WalkPhase::Complete;
                }
                WalkPhase::Complete => return Ok(true),
            }
        }
    }

    fn gc_process_task(&self, walk: &mut GcWalk, task: Task, budget: &mut Budget) -> Result<bool> {
        match task {
            Task::ProbeRoot { node } => self.gc_probe_root(walk, node, budget),
            Task::Document {
                node,
                expected,
                platform,
                depth,
                root,
            } => {
                let digest = walk.catalog[usize::from(node)].clone();
                let size = self.gc_blob_size(&digest)?;
                if !budget.document(size, self.limits.max_metadata)? {
                    return Ok(false);
                }
                ensure!(depth <= 8, "OCI graph limit exceeded");
                walk.mark_expanded(node)?;
                let bytes = self.metadata(&digest)?;
                oci::gc_walk::expand_document(
                    self, walk, node, expected, platform, depth, root, &bytes,
                )?;
                Ok(true)
            }
            Task::Descriptor(task) => {
                if !budget.item() {
                    return Ok(false);
                }
                oci::gc_walk::verify_descriptor(self, walk, task)
            }
            Task::Config {
                node,
                manifest,
                layer_count,
                platform,
            } => {
                let digest = walk.catalog[usize::from(node)].clone();
                let size = self.gc_blob_size(&digest)?;
                if !budget.document(size, self.limits.max_metadata)? {
                    return Ok(false);
                }
                let bytes = self.metadata(&digest)?;
                oci::gc_walk::verify_config(walk, manifest, layer_count, platform, &bytes)?;
                Ok(true)
            }
        }
    }

    fn gc_probe_root(&self, walk: &mut GcWalk, node: u16, budget: &mut Budget) -> Result<bool> {
        let digest = walk.catalog[usize::from(node)].clone();
        let root = self.db.get::<state::Root>("roots", digest.as_str())?;
        let size = self.gc_blob_size(&digest)?;
        ensure!(size <= self.limits.max_blob, "reachable blob exceeds limit");
        if root.is_some() {
            if !budget.item() {
                return Ok(false);
            }
            walk.schedule_image(node, self.limits.max_graph)?;
            walk.pending.push(Task::Document {
                node,
                expected: None,
                platform: None,
                depth: 0,
                root: true,
            });
            return Ok(true);
        }
        if size > self.limits.max_metadata {
            if !budget.item() {
                return Ok(false);
            }
            self.gc_raw_root(walk, &digest)?;
            return Ok(true);
        }
        if !budget.document(size, self.limits.max_metadata)? {
            return Ok(false);
        }
        let bytes = self.metadata(&digest)?;
        let looks_oci = serde_json::from_slice::<serde_json::Value>(&bytes).is_ok_and(|value| {
            value.get("schemaVersion").and_then(|v| v.as_u64()) == Some(2)
                && (value.get("manifests").is_some()
                    || (value.get("config").is_some() && value.get("layers").is_some()))
        });
        if looks_oci {
            walk.schedule_image(node, self.limits.max_graph)?;
            walk.mark_expanded(node)?;
            oci::gc_walk::expand_document(self, walk, node, None, None, 0, true, &bytes)?;
        } else {
            self.gc_raw_root(walk, &digest)?;
        }
        Ok(true)
    }

    fn gc_raw_root(&self, walk: &mut GcWalk, digest: &ArtifactDigest) -> Result<()> {
        let blob = self
            .db
            .get::<state::Blob>("blobs", digest.as_str())?
            .ok_or_else(|| anyhow::anyhow!("missing blob"))?;
        let file = filesystem::read(&self.blobs, digest.hex())?;
        ensure!(
            file.metadata()?.len() == blob.size,
            "reachable blob size mismatch"
        );
        walk.raw = true;
        walk.phase = WalkPhase::Edges;
        walk.cursor = 0;
        Ok(())
    }

    fn gc_blob_size(&self, digest: &ArtifactDigest) -> Result<u64> {
        self.db
            .get::<state::Blob>("blobs", digest.as_str())?
            .map(|blob| blob.size)
            .ok_or_else(|| anyhow::anyhow!("missing blob"))
    }

    fn gc_verify_outgoing(&self, walk: &GcWalk, parent: usize) -> Result<()> {
        let parent_digest = &walk.catalog[parent];
        let expected: BTreeSet<_> = walk
            .edges
            .iter()
            .filter(|(from, _)| usize::from(*from) == parent)
            .map(|(_, to)| walk.catalog[usize::from(*to)].clone())
            .collect();
        let actual: BTreeSet<_> = self
            .db
            .get::<Vec<ArtifactDigest>>("edges", parent_digest.as_str())?
            .unwrap_or_default()
            .into_iter()
            .collect();
        ensure!(
            actual.len() <= self.limits.max_graph,
            "persisted graph exceeds bound"
        );
        ensure!(actual == expected, "persisted OCI reachability mismatch");
        Ok(())
    }

    fn gc_verify_image(&self, walk: &GcWalk, index: usize) -> Result<()> {
        let image = &walk.images[index];
        let digest = &walk.catalog[usize::from(image.manifest)];
        let expected = state::Root {
            kind: "manifest".into(),
            platform: Some(image.platform.clone()),
        };
        ensure!(
            self.db
                .get::<state::Root>("roots", digest.as_str())?
                .as_ref()
                == Some(&expected),
            "OCI manifest classification mismatch"
        );
        Ok(())
    }

    fn gc_verify_root_classification(&self, walk: &GcWalk) -> Result<()> {
        ensure!(
            walk.scheduled == walk.expanded,
            "OCI graph discovery is incomplete"
        );
        let Some(digest) = walk.digest.as_ref() else {
            return Ok(());
        };
        if walk.root_kind.is_some() {
            ensure!(!walk.images.is_empty(), "OCI graph has no runnable images");
        }
        let expected = match walk.root_kind {
            Some(GraphKind::Manifest) => {
                let image = walk
                    .images
                    .iter()
                    .find(|image| walk.catalog[usize::from(image.manifest)] == *digest)
                    .ok_or_else(|| anyhow::anyhow!("root manifest classification missing"))?;
                state::Root {
                    kind: "manifest".into(),
                    platform: Some(image.platform.clone()),
                }
            }
            Some(GraphKind::Index) => state::Root {
                kind: "oci".into(),
                platform: None,
            },
            None => {
                ensure!(walk.raw, "OCI graph has no runnable images");
                return Ok(());
            }
        };
        ensure!(
            self.db
                .get::<state::Root>("roots", digest.as_str())?
                .as_ref()
                == Some(&expected),
            "OCI root classification mismatch"
        );
        Ok(())
    }

    pub(in crate::gc) fn gc_walk_complete(&self, walk: &GcWalk) -> bool {
        walk.complete()
    }

    pub(in crate::gc) fn gc_walk_nodes(&self, walk: &GcWalk) -> Vec<ArtifactDigest> {
        walk.nodes()
    }
}
