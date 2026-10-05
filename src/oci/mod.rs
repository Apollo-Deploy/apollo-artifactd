//! OCI types come from oci-spec; this module enforces graph integrity policy.
mod admission;
mod archive;
pub(crate) mod gc_walk;
mod graph;
mod reachability;
use crate::Store;
use anyhow::{Result, ensure};
use artifactd_protocol::{ArtifactDigest, Platform};
use oci_spec::image::{ImageConfiguration, ImageManifest, MediaType};

pub struct Image {
    pub digest: ArtifactDigest,
    /// Exact byte length of the verified manifest document in the CAS.
    pub manifest_size: u64,
    pub manifest: ImageManifest,
    pub config: ImageConfiguration,
}

impl Store {
    pub fn resolve(&self, digest: &ArtifactDigest, platform: &Platform) -> Result<Image> {
        ensure!(platform.validate(), "unsupported platform");
        let mut graph = self.graph(digest)?;
        let chosen = graph::select(&graph, platform)?.digest.clone();
        graph
            .images
            .remove(&chosen)
            .ok_or_else(|| anyhow::anyhow!("selected image missing"))
    }

    pub(crate) fn verify_graph_if_known(&self, digest: &ArtifactDigest) -> Result<()> {
        self.verified_graph_nodes(digest).map(|_| ())
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
/// Return generic facts for a graph that has passed OCI descriptor and
/// metadata-relationship validation. `verified` covers that graph scope;
/// layer tar DiffID verification remains part of rootfs preparation.
pub fn facts(source: &ArtifactDigest, image: &Image, platform: &Platform) -> serde_json::Value {
    let config = image.manifest.config();
    let layers = image.manifest.layers();
    serde_json::json!({
        "artifact_digest": source,
        "manifest_digest": image.digest,
        "manifest_size": image.manifest_size,
        "config_digest": config.digest(),
        "config": {
            "digest": config.digest(),
            "media_type": config.media_type().to_string(),
            "size": config.size(),
        },
        "layer_digests": layers.iter().map(|l| l.digest().to_string()).collect::<Vec<_>>(),
        "layers": layers.iter().map(|layer| serde_json::json!({
            "digest": layer.digest(),
            "media_type": layer.media_type().to_string(),
            "size": layer.size(),
        })).collect::<Vec<_>>(),
        "platform": platform,
        "verified": true,
    })
}
