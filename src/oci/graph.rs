use super::{Image, Store, layer_type, platform_of};
use anyhow::{Result, bail, ensure};
use artifactd_protocol::{ArtifactDigest, Platform};
use oci_spec::image::{Descriptor, ImageConfiguration, ImageIndex, ImageManifest, MediaType};
use std::collections::{BTreeMap, BTreeSet};

pub(super) struct Graph {
    pub(super) nodes: BTreeSet<ArtifactDigest>,
    pub(super) edges: BTreeSet<(ArtifactDigest, ArtifactDigest)>,
    pub(super) images: BTreeMap<ArtifactDigest, Image>,
}

impl Store {
    pub(super) fn verified_graph_nodes(
        &self,
        digest: &ArtifactDigest,
    ) -> Result<BTreeSet<ArtifactDigest>> {
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
            return Ok(nodes);
        }
        self.open_blob(digest)?;
        ensure!(
            self.db
                .get::<Vec<ArtifactDigest>>("edges", digest.as_str())?
                .is_none_or(|children| children.is_empty()),
            "raw blob has persisted graph edges"
        );
        Ok(BTreeSet::from([digest.clone()]))
    }

    pub(super) fn graph(&self, root: &ArtifactDigest) -> Result<Graph> {
        self.graph_with_descriptor_verification(root, true)
    }

    /// Rebuilds the OCI descriptor graph for GC while verifying small metadata
    /// blobs normally and checking only ownership and size for layer blobs.
    /// GC must detect damaged graph metadata without hashing multi-gigabyte
    /// layers on every bounded collection pass.
    pub(super) fn graph_for_gc(&self, root: &ArtifactDigest) -> Result<Graph> {
        self.graph_with_descriptor_verification(root, false)
    }

    fn graph_with_descriptor_verification(
        &self,
        root: &ArtifactDigest,
        verify_descriptor_bytes: bool,
    ) -> Result<Graph> {
        let mut graph = Graph {
            nodes: BTreeSet::new(),
            edges: BTreeSet::new(),
            images: BTreeMap::new(),
        };
        self.visit(root, None, &mut graph, 0, verify_descriptor_bytes)?;
        ensure!(!graph.images.is_empty(), "OCI graph has no runnable images");
        Ok(graph)
    }

    fn visit(
        &self,
        digest: &ArtifactDigest,
        declared: Option<&MediaType>,
        graph: &mut Graph,
        depth: usize,
        verify_descriptor_bytes: bool,
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
                let child =
                    self.descriptor_with_verification(descriptor, verify_descriptor_bytes)?;
                graph.edges.insert((digest.clone(), child.clone()));
                self.visit(
                    &child,
                    Some(descriptor.media_type()),
                    graph,
                    depth + 1,
                    verify_descriptor_bytes,
                )?;
                if let Some(platform) = descriptor.platform() {
                    let image = graph
                        .images
                        .get(&child)
                        .ok_or_else(|| anyhow::anyhow!("platform on non-manifest descriptor"))?;
                    ensure!(
                        platform.os().to_string() == image.config.os().to_string()
                            && platform.architecture().to_string()
                                == image.config.architecture().to_string()
                            && platform.variant() == image.config.variant(),
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
            let config_digest =
                self.descriptor_with_verification(manifest.config(), verify_descriptor_bytes)?;
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
            let platform = platform_of(&config);
            ensure!(platform.validate(), "unsupported image platform");
            for layer in manifest.layers() {
                ensure!(
                    layer_type(layer.media_type()),
                    "unsupported layer media type"
                );
                let child = self.descriptor_with_verification(layer, verify_descriptor_bytes)?;
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
                    manifest_size: bytes.len() as u64,
                    manifest,
                    config,
                },
            );
        }
        Ok(())
    }

    pub(crate) fn descriptor(&self, descriptor: &Descriptor) -> Result<ArtifactDigest> {
        self.descriptor_with_verification(descriptor, true)
    }

    fn descriptor_with_verification(
        &self,
        descriptor: &Descriptor,
        verify_bytes: bool,
    ) -> Result<ArtifactDigest> {
        ensure!(
            descriptor.urls().as_ref().is_none_or(Vec::is_empty) && descriptor.data().is_none(),
            "external/inline descriptor unsupported"
        );
        let digest: ArtifactDigest = descriptor.digest().to_string().parse()?;
        // Must be an admitted CAS blob, not merely an untracked filename.
        let file = if verify_bytes {
            self.open_blob(&digest)?
        } else {
            let blob = self
                .db
                .get::<crate::state::Blob>("blobs", digest.as_str())?
                .ok_or_else(|| anyhow::anyhow!("descriptor blob is not admitted"))?;
            ensure!(
                blob.size <= self.limits.max_blob,
                "descriptor blob exceeds limit"
            );
            let file = crate::filesystem::read(&self.blobs, digest.hex())?;
            ensure!(
                file.metadata()?.len() == blob.size,
                "descriptor blob size mismatch"
            );
            file
        };
        ensure!(
            file.metadata()?.len() == descriptor.size(),
            "descriptor size mismatch"
        );
        Ok(digest)
    }
}

pub(super) fn select<'a>(graph: &'a Graph, platform: &Platform) -> Result<&'a Image> {
    let matches: Vec<_> = graph
        .images
        .values()
        .filter(|image| platform_of(&image.config) == *platform)
        .collect();
    if matches.len() != 1 {
        bail!("platform is missing or ambiguous");
    }
    Ok(matches[0])
}
