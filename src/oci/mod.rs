//! OCI types come from oci-spec; this module enforces graph integrity policy.
mod archive;
use crate::Store;
use anyhow::{Result, bail, ensure};
use artifactd_protocol::{ArtifactDigest, Platform};
use oci_spec::image::{Descriptor, ImageConfiguration, ImageIndex, ImageManifest, MediaType};
use std::collections::{BTreeMap, BTreeSet};

pub struct Image {
    pub digest: ArtifactDigest,
    pub manifest: ImageManifest,
    pub config: ImageConfiguration,
}

struct Graph {
    nodes: BTreeSet<ArtifactDigest>,
    edges: BTreeSet<(ArtifactDigest, ArtifactDigest)>,
    images: BTreeMap<ArtifactDigest, Image>,
}

impl Store {
    pub fn admit_oci(
        &mut self,
        digest: &ArtifactDigest,
        platform: &Platform,
    ) -> Result<serde_json::Value> {
        ensure!(platform.validate(), "unsupported platform");
        let graph = self.graph(digest)?;
        let image = select(&graph, platform)?;
        let facts = facts(digest, image, platform);
        self.db.transaction(|tx| {
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
            Ok(())
        })?;
        Ok(facts)
    }

    pub fn resolve(&self, digest: &ArtifactDigest, platform: &Platform) -> Result<Image> {
        ensure!(platform.validate(), "unsupported platform");
        let mut graph = self.graph(digest)?;
        let chosen = select(&graph, platform)?.digest.clone();
        graph
            .images
            .remove(&chosen)
            .ok_or_else(|| anyhow::anyhow!("selected image missing"))
    }

    pub(crate) fn verify_graph_if_known(&self, digest: &ArtifactDigest) -> Result<()> {
        let known = self
            .db
            .get::<crate::state::Root>("roots", digest.as_str())?
            .is_some();
        let size = self
            .db
            .get::<crate::state::Blob>("blobs", digest.as_str())?
            .ok_or_else(|| anyhow::anyhow!("missing blob"))?
            .size;
        let looks_oci = if !known && size <= self.limits.max_metadata {
            let bytes = self.metadata(digest)?;
            serde_json::from_slice::<serde_json::Value>(&bytes).is_ok_and(|value| {
                value.get("schemaVersion").and_then(|v| v.as_u64()) == Some(2)
                    && (value.get("manifests").is_some()
                        || (value.get("config").is_some() && value.get("layers").is_some()))
            })
        } else {
            false
        };
        // A lost classification row must not turn a pinned OCI graph into a raw blob.
        if known || looks_oci {
            let graph = self.graph(digest)?;
            let mut persisted = BTreeSet::new();
            for parent in &graph.nodes {
                if let Some(children) = self
                    .db
                    .get::<Vec<ArtifactDigest>>("edges", parent.as_str())?
                {
                    for child in children {
                        persisted.insert((parent.clone(), child));
                    }
                    ensure!(
                        persisted.len() <= self.limits.max_graph,
                        "persisted graph exceeds bound"
                    );
                }
            }
            ensure!(
                persisted == graph.edges,
                "persisted OCI reachability mismatch"
            );
        }
        Ok(())
    }

    fn graph(&self, root: &ArtifactDigest) -> Result<Graph> {
        let mut graph = Graph {
            nodes: BTreeSet::new(),
            edges: BTreeSet::new(),
            images: BTreeMap::new(),
        };
        self.visit(root, None, &mut graph, 0)?;
        ensure!(!graph.images.is_empty(), "OCI graph has no runnable images");
        Ok(graph)
    }

    fn visit(
        &self,
        digest: &ArtifactDigest,
        declared: Option<&MediaType>,
        graph: &mut Graph,
        depth: usize,
    ) -> Result<()> {
        ensure!(
            depth <= 8 && graph.nodes.len() < self.limits.max_graph,
            "OCI graph limit exceeded"
        );
        ensure!(
            graph.nodes.insert(digest.clone()),
            "OCI graph cycle or duplicate manifest"
        );
        let bytes = self.metadata(digest)?;
        let value: serde_json::Value = serde_json::from_slice(&bytes)?;
        let object = value
            .as_object()
            .ok_or_else(|| anyhow::anyhow!("OCI metadata must be an object"))?;
        if object.contains_key("manifests") {
            ensure!(
                !object.contains_key("layers") && !object.contains_key("config"),
                "ambiguous OCI document"
            );
            ensure!(
                declared.is_none_or(|t| *t == MediaType::ImageIndex),
                "descriptor media type mismatch"
            );
            let index: ImageIndex = serde_json::from_slice(&bytes)?;
            ensure!(
                index.schema_version() == 2
                    && index
                        .media_type()
                        .as_ref()
                        .is_none_or(|v| *v == MediaType::ImageIndex),
                "invalid index type/version"
            );
            ensure!(
                !index.manifests().is_empty() && index.manifests().len() <= self.limits.max_graph,
                "invalid index size"
            );
            for descriptor in index.manifests() {
                ensure!(
                    matches!(
                        descriptor.media_type(),
                        MediaType::ImageManifest | MediaType::ImageIndex
                    ),
                    "unsupported index descriptor"
                );
                let child = self.descriptor(descriptor)?;
                graph.edges.insert((digest.clone(), child.clone()));
                self.visit(&child, Some(descriptor.media_type()), graph, depth + 1)?;
                if let Some(p) = descriptor.platform() {
                    let image = graph
                        .images
                        .get(&child)
                        .ok_or_else(|| anyhow::anyhow!("platform on non-manifest descriptor"))?;
                    ensure!(
                        p.os().to_string() == image.config.os().to_string()
                            && p.architecture().to_string()
                                == image.config.architecture().to_string()
                            && p.variant() == image.config.variant(),
                        "platform substitution"
                    );
                }
            }
        } else {
            ensure!(
                declared.is_none_or(|t| *t == MediaType::ImageManifest),
                "descriptor media type mismatch"
            );
            let manifest: ImageManifest = serde_json::from_slice(&bytes)?;
            ensure!(
                manifest.schema_version() == 2
                    && manifest
                        .media_type()
                        .as_ref()
                        .is_none_or(|v| *v == MediaType::ImageManifest),
                "invalid manifest type/version"
            );
            ensure!(manifest.subject().is_none(), "subject graphs unsupported");
            ensure!(
                *manifest.config().media_type() == MediaType::ImageConfig,
                "invalid config media type"
            );
            ensure!(
                manifest.layers().len() <= self.limits.max_graph,
                "too many layers"
            );
            let config_digest = self.descriptor(manifest.config())?;
            graph.edges.insert((digest.clone(), config_digest.clone()));
            let config: ImageConfiguration =
                serde_json::from_slice(&self.metadata(&config_digest)?)?;
            ensure!(
                config.rootfs().typ() == "layers"
                    && config.rootfs().diff_ids().len() == manifest.layers().len(),
                "invalid DiffID relationship"
            );
            for diff in config.rootfs().diff_ids() {
                let _: ArtifactDigest = diff.parse()?;
            }
            let p = platform_of(&config);
            ensure!(p.validate(), "unsupported image platform");
            for layer in manifest.layers() {
                ensure!(
                    layer_type(layer.media_type()),
                    "unsupported layer media type"
                );
                let child = self.descriptor(layer)?;
                graph.edges.insert((digest.clone(), child));
                ensure!(
                    graph.edges.len() <= self.limits.max_graph,
                    "OCI graph limit exceeded"
                );
            }
            graph.images.insert(
                digest.clone(),
                Image {
                    digest: digest.clone(),
                    manifest,
                    config,
                },
            );
        }
        Ok(())
    }

    pub(crate) fn descriptor(&self, descriptor: &Descriptor) -> Result<ArtifactDigest> {
        ensure!(
            descriptor.urls().as_ref().is_none_or(Vec::is_empty) && descriptor.data().is_none(),
            "external/inline descriptor unsupported"
        );
        let digest: ArtifactDigest = descriptor.digest().to_string().parse()?;
        // Must be an admitted CAS blob, not merely an untracked filename.
        let file = self.open_blob(&digest)?;
        ensure!(
            file.metadata()?.len() == descriptor.size(),
            "descriptor size mismatch"
        );
        Ok(digest)
    }
}

pub fn platform_of(config: &ImageConfiguration) -> Platform {
    Platform {
        os: config.os().to_string(),
        architecture: config.architecture().to_string(),
        variant: config.variant().clone(),
    }
}
pub fn layer_type(media: &MediaType) -> bool {
    matches!(
        media.to_string().as_str(),
        "application/vnd.oci.image.layer.v1.tar"
            | "application/vnd.oci.image.layer.v1.tar+gzip"
            | "application/vnd.oci.image.layer.v1.tar+zstd"
    )
}
fn select<'a>(graph: &'a Graph, platform: &Platform) -> Result<&'a Image> {
    let matches: Vec<_> = graph
        .images
        .values()
        .filter(|i| platform_of(&i.config) == *platform)
        .collect();
    if matches.len() != 1 {
        bail!("platform is missing or ambiguous");
    }
    Ok(matches[0])
}
pub fn facts(source: &ArtifactDigest, image: &Image, platform: &Platform) -> serde_json::Value {
    serde_json::json!({"artifact_digest": source, "manifest_digest": image.digest, "config_digest": image.manifest.config().digest().to_string(), "layer_digests": image.manifest.layers().iter().map(|l| l.digest().to_string()).collect::<Vec<_>>(), "platform": platform})
}
