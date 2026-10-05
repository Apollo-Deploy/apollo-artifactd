//! Single-document OCI graph expansion used by the bounded GC walker.
use crate::{Store, gc::walk};
use anyhow::{Result, ensure};
use artifactd_protocol::{ArtifactDigest, Platform};
use oci_spec::image::{Descriptor, ImageConfiguration, ImageIndex, ImageManifest, MediaType};

pub(crate) fn expand_document(
    store: &Store,
    state: &mut walk::GcWalk,
    node: u16,
    expected: Option<walk::GraphKind>,
    platform: Option<Platform>,
    depth: u8,
    root: bool,
    bytes: &[u8],
) -> Result<()> {
    let value: serde_json::Value = serde_json::from_slice(bytes)?;
    let object = value
        .as_object()
        .ok_or_else(|| anyhow::anyhow!("OCI metadata must be an object"))?;
    if object.contains_key("manifests") {
        ensure!(
            !object.contains_key("layers") && !object.contains_key("config"),
            "ambiguous OCI document"
        );
        ensure!(
            expected.is_none_or(|kind| kind == walk::GraphKind::Index),
            "descriptor media type mismatch"
        );
        ensure!(platform.is_none(), "platform on non-manifest descriptor");
        let index: ImageIndex = serde_json::from_slice(bytes)?;
        ensure!(
            index.schema_version() == 2
                && index
                    .media_type()
                    .as_ref()
                    .is_none_or(|media| *media == MediaType::ImageIndex),
            "invalid index type/version"
        );
        ensure!(
            !index.manifests().is_empty() && index.manifests().len() <= store.limits.max_graph,
            "invalid index size"
        );
        if root {
            state.root_kind = Some(walk::GraphKind::Index);
        }
        for descriptor in index.manifests() {
            let child_kind = match descriptor.media_type() {
                MediaType::ImageManifest => walk::GraphKind::Manifest,
                MediaType::ImageIndex => walk::GraphKind::Index,
                _ => anyhow::bail!("unsupported index descriptor"),
            };
            let (digest, size) = descriptor_parts(descriptor)?;
            let platform = descriptor.platform().as_ref().map(platform_from_oci);
            ensure!(
                platform.as_ref().is_none_or(Platform::validate),
                "unsupported descriptor platform"
            );
            let child_depth = depth
                .checked_add(1)
                .ok_or_else(|| anyhow::anyhow!("OCI graph depth overflow"))?;
            ensure!(child_depth <= 8, "OCI graph limit exceeded");
            state.add_descriptor(
                node,
                digest,
                size,
                walk::DescriptorTarget::Image {
                    kind: child_kind,
                    platform,
                    depth: child_depth,
                },
                store.limits.max_graph,
            )?;
        }
        return Ok(());
    }

    ensure!(
        expected.is_none_or(|kind| kind == walk::GraphKind::Manifest),
        "descriptor media type mismatch"
    );
    let manifest: ImageManifest = serde_json::from_slice(bytes)?;
    ensure!(
        manifest.schema_version() == 2
            && manifest
                .media_type()
                .as_ref()
                .is_none_or(|media| *media == MediaType::ImageManifest),
        "invalid manifest type/version"
    );
    ensure!(manifest.subject().is_none(), "subject graphs unsupported");
    ensure!(
        *manifest.config().media_type() == MediaType::ImageConfig,
        "invalid config media type"
    );
    ensure!(
        manifest.layers().len() <= store.limits.max_graph,
        "too many layers"
    );
    if root {
        state.root_kind = Some(walk::GraphKind::Manifest);
    }
    let (config_digest, config_size) = descriptor_parts(manifest.config())?;
    state.add_descriptor(
        node,
        config_digest,
        config_size,
        walk::DescriptorTarget::Config {
            manifest: node,
            layer_count: u32::try_from(manifest.layers().len())?,
            platform,
        },
        store.limits.max_graph,
    )?;
    for layer in manifest.layers() {
        ensure!(
            crate::oci::layer_type(layer.media_type()),
            "unsupported layer media type"
        );
        let (digest, size) = descriptor_parts(layer)?;
        state.add_descriptor(
            node,
            digest,
            size,
            walk::DescriptorTarget::Layer,
            store.limits.max_graph,
        )?;
    }
    Ok(())
}

pub(crate) fn verify_descriptor(
    store: &Store,
    state: &mut walk::GcWalk,
    descriptor: walk::DescriptorTask,
) -> Result<bool> {
    let digest = &state.catalog[usize::from(descriptor.child)];
    let blob = store
        .db
        .get::<crate::state::Blob>("blobs", digest.as_str())?
        .ok_or_else(|| anyhow::anyhow!("descriptor blob is not admitted"))?;
    ensure!(
        blob.size <= store.limits.max_blob,
        "descriptor blob exceeds limit"
    );
    let file = crate::filesystem::read(&store.blobs, digest.hex())?;
    ensure!(
        file.metadata()?.len() == blob.size && blob.size == descriptor.size,
        "descriptor size mismatch"
    );
    match descriptor.target {
        walk::DescriptorTarget::Image {
            kind,
            platform,
            depth,
        } => state.push(walk::Task::Document {
            node: descriptor.child,
            expected: Some(kind),
            platform,
            depth,
            root: false,
        }),
        walk::DescriptorTarget::Config {
            manifest,
            layer_count,
            platform,
        } => state.push(walk::Task::Config {
            node: descriptor.child,
            manifest,
            layer_count,
            platform,
        }),
        walk::DescriptorTarget::Layer => {}
    }
    Ok(true)
}

pub(crate) fn verify_config(
    state: &mut walk::GcWalk,
    manifest: u16,
    layer_count: u32,
    expected_platform: Option<Platform>,
    bytes: &[u8],
) -> Result<()> {
    let config: ImageConfiguration = serde_json::from_slice(bytes)?;
    ensure!(
        config.rootfs().typ() == "layers"
            && config.rootfs().diff_ids().len() == usize::try_from(layer_count)?,
        "invalid DiffID relationship"
    );
    for diff in config.rootfs().diff_ids() {
        let _: ArtifactDigest = diff.parse()?;
    }
    let platform = crate::oci::platform_of(&config);
    ensure!(platform.validate(), "unsupported image platform");
    ensure!(
        expected_platform
            .as_ref()
            .is_none_or(|expected| expected == &platform),
        "platform substitution"
    );
    state
        .images
        .push(walk::ImagePlatform { manifest, platform });
    Ok(())
}

fn descriptor_parts(descriptor: &Descriptor) -> Result<(ArtifactDigest, u64)> {
    ensure!(
        descriptor.urls().as_ref().is_none_or(Vec::is_empty) && descriptor.data().is_none(),
        "external/inline descriptor unsupported"
    );
    Ok((descriptor.digest().to_string().parse()?, descriptor.size()))
}

fn platform_from_oci(platform: &oci_spec::image::Platform) -> Platform {
    Platform {
        os: platform.os().to_string(),
        architecture: platform.architecture().to_string(),
        variant: platform.variant().clone(),
    }
}
