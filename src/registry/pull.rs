use super::{Credentials, Registry, credentials::reference, remote};
use crate::Store;
use anyhow::{Result, ensure};
use artifactd_protocol::{ArtifactDigest, Platform};
use oci_client::{Client, Reference, manifest::OciDescriptor, secrets::RegistryAuth};
use oci_spec::image::{Descriptor, ImageIndex, ImageManifest, MediaType};
use std::collections::BTreeMap;
use tokio_util::io::{StreamReader, SyncIoBridge};

const ACCEPT: &[&str] = &[
    "application/vnd.oci.image.index.v1+json",
    "application/vnd.oci.image.manifest.v1+json",
];

impl Registry {
    pub fn pull(
        &self,
        store: &mut Store,
        value: &str,
        platform: &Platform,
        credentials: Credentials,
    ) -> Result<serde_json::Value> {
        self.pull_with_pin(store, value, platform, credentials, None)
    }

    pub fn pull_pinned(
        &self,
        store: &mut Store,
        value: &str,
        platform: &Platform,
        credentials: Credentials,
        pin: &str,
    ) -> Result<serde_json::Value> {
        artifactd_protocol::PinId::try_from(pin.to_owned()).map_err(anyhow::Error::msg)?;
        self.pull_with_pin(store, value, platform, credentials, Some(pin))
    }

    fn pull_with_pin(
        &self,
        store: &mut Store,
        value: &str,
        platform: &Platform,
        credentials: Credentials,
        pin: Option<&str>,
    ) -> Result<serde_json::Value> {
        ensure!(platform.validate(), "unsupported platform");
        let image = reference(value, true)?;
        let root: ArtifactDigest = image.digest().expect("validated digest").parse()?;
        // A valid local graph is already the immutable requested result.
        if store.resolve(&root, platform).is_ok() {
            return store.admit_oci_with_pin(&root, platform, pin);
        }
        let auth = credentials.auth(&image)?;
        let client = self.client(&credentials, store.limits.max_metadata)?;
        let mut queue = vec![(root.clone(), None::<Descriptor>, 0usize)];
        let mut seen = BTreeMap::<ArtifactDigest, Option<Descriptor>>::new();
        while let Some((digest, descriptor, depth)) = queue.pop() {
            ensure!(
                depth <= 8 && seen.len() < store.limits.max_graph,
                "registry graph limit exceeded"
            );
            if let Some(previous) = seen.get(&digest) {
                ensure!(previous == &descriptor, "conflicting registry descriptors");
                continue;
            }
            let manifest = descriptor.as_ref().is_none_or(|d| {
                matches!(
                    d.media_type(),
                    MediaType::ImageIndex | MediaType::ImageManifest
                )
            });
            if manifest {
                let pinned = image.clone_with_digest(digest.as_str().to_owned());
                let bytes = self.manifest(&client, &pinned, &auth)?;
                ensure!(
                    bytes.len() as u64 <= store.limits.max_metadata,
                    "registry metadata exceeds limit"
                );
                if let Some(expected) = &descriptor {
                    ensure!(
                        bytes.len() as u64 == expected.size(),
                        "registry descriptor size mismatch"
                    );
                }
                let size = bytes.len() as u64;
                store.import_blob(&mut std::io::Cursor::new(bytes.clone()), &digest, size)?;
                let value: serde_json::Value = serde_json::from_slice(&bytes)?;
                let children = if value.get("manifests").is_some() {
                    let index: ImageIndex = serde_json::from_slice(&bytes)?;
                    ensure!(
                        index.schema_version() == 2,
                        "invalid registry index version"
                    );
                    index.manifests().clone()
                } else {
                    let manifest: ImageManifest = serde_json::from_slice(&bytes)?;
                    ensure!(
                        manifest.schema_version() == 2,
                        "invalid registry manifest version"
                    );
                    let mut children = manifest.layers().clone();
                    children.push(manifest.config().clone());
                    children
                };
                ensure!(
                    children.len() <= store.limits.max_graph
                        && queue.len() + children.len() <= store.limits.max_graph,
                    "registry graph limit exceeded"
                );
                for child in children {
                    validate(&child, store)?;
                    queue.push((child.digest().to_string().parse()?, Some(child), depth + 1));
                }
            } else {
                let descriptor = descriptor.as_ref().expect("blob descriptor");
                if store.descriptor(descriptor).is_err() {
                    self.blob(&client, &image, store, descriptor, &digest)?;
                }
            }
            seen.insert(digest, descriptor);
        }
        // Independently verify the complete graph using artifactd policy. Registry
        // responses are transport inputs, never authoritative graph facts.
        store.admit_oci_with_pin(&root, platform, pin)
    }
    fn manifest(
        &self,
        client: &Client,
        image: &Reference,
        auth: &RegistryAuth,
    ) -> Result<bytes::Bytes> {
        let mut outcome = None;
        for attempt in 0..3 {
            match remote(
                self.runtime
                    .block_on(client.pull_manifest_raw(image, auth, ACCEPT)),
            ) {
                Ok((bytes, _)) => return Ok(bytes),
                Err(error) => outcome = Some(error),
            }
            self.backoff(attempt);
        }
        Err(outcome.expect("three attempts"))
    }
    fn blob(
        &self,
        client: &Client,
        image: &Reference,
        store: &mut Store,
        descriptor: &Descriptor,
        digest: &ArtifactDigest,
    ) -> Result<()> {
        let transport: OciDescriptor = serde_json::from_value(serde_json::to_value(descriptor)?)?;
        for attempt in 0..3 {
            let stream = remote(
                self.runtime
                    .block_on(client.pull_blob_stream(image, &transport)),
            );
            if let Ok(stream) = stream {
                ensure!(
                    stream
                        .content_length
                        .is_none_or(|size| size == descriptor.size()),
                    "registry blob size mismatch"
                );
                let reader = StreamReader::new(stream.stream);
                let mut bridge =
                    SyncIoBridge::new_with_handle(reader, self.runtime.handle().clone());
                if store
                    .import_blob(&mut bridge, digest, descriptor.size())
                    .is_ok()
                {
                    return Ok(());
                }
            }
            self.backoff(attempt);
        }
        anyhow::bail!("registry blob transfer or integrity verification failed")
    }
    pub(super) fn backoff(&self, attempt: usize) {
        if attempt < 2 {
            self.runtime.block_on(async {
                tokio::time::sleep(std::time::Duration::from_millis(100 << attempt)).await;
            });
        }
    }
}

fn validate(descriptor: &Descriptor, store: &Store) -> Result<()> {
    let _: ArtifactDigest = descriptor.digest().to_string().parse()?;
    ensure!(
        descriptor.size() <= store.limits.max_blob,
        "registry descriptor exceeds limit"
    );
    ensure!(
        descriptor.urls().as_ref().is_none_or(Vec::is_empty) && descriptor.data().is_none(),
        "external/inline registry descriptor rejected"
    );
    ensure!(
        matches!(
            descriptor.media_type(),
            MediaType::ImageIndex | MediaType::ImageManifest | MediaType::ImageConfig
        ) || crate::oci::layer_type(descriptor.media_type()),
        "unsupported registry media type"
    );
    Ok(())
}
