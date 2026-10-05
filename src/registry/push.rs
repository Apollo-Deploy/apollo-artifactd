use super::{Credentials, Registry, credentials::reference, remote};
use crate::Store;
use anyhow::{Result, ensure};
use artifactd_protocol::ArtifactDigest;
use futures_util::TryStreamExt;
use oci_client::{Client, Reference, RegistryOperation};
use oci_spec::image::{Descriptor, ImageIndex, ImageManifest, MediaType};
use std::collections::BTreeSet;
use tokio_util::io::ReaderStream;

struct Manifest {
    digest: ArtifactDigest,
    media_type: String,
}
impl Registry {
    pub fn push(
        &self,
        store: &Store,
        digest: &ArtifactDigest,
        value: &str,
        credentials: Credentials,
    ) -> Result<serde_json::Value> {
        let destination = reference(value, false)?;
        ensure!(
            destination.digest().is_none_or(|v| v == digest.as_str()),
            "push destination digest mismatch"
        );
        // A raw CAS blob is never an OCI publication source. Admission records
        // the verified graph and its descriptor relationships before any
        // registry authentication or transfer can have an external effect.
        ensure!(
            store
                .db
                .get::<crate::state::Root>("roots", digest.as_str())?
                .is_some(),
            "push requires admitted OCI root"
        );
        store.verify_graph_if_known(digest)?;
        let client = self.client(&credentials, store.limits.max_metadata)?;
        let auth = credentials.auth(&destination)?;
        remote(
            self.runtime
                .block_on(client.auth(&destination, &auth, RegistryOperation::Push)),
        )?;
        let mut manifests = Vec::new();
        let mut blobs = BTreeSet::new();
        let mut seen = BTreeSet::new();
        collect(store, digest, &mut seen, &mut manifests, &mut blobs, 0)?;
        // Config and layers first; child manifests precede their index.
        for blob in &blobs {
            self.push_blob(&client, &destination, store, blob)?;
        }
        for manifest in &manifests {
            let pinned = destination.clone_with_digest(manifest.digest.as_str().to_owned());
            self.push_manifest(&client, &pinned, manifest, store, &auth)?;
        }
        if destination.tag().is_some() {
            self.push_manifest(
                &client,
                &destination,
                manifests.last().expect("root manifest"),
                store,
                &auth,
            )?;
            let root = manifests.last().expect("root manifest");
            let (observed, _) = remote(self.runtime.block_on(client.pull_manifest_raw(
                &destination,
                &auth,
                &[&root.media_type],
            )))?;
            ensure!(
                observed.as_ref() == store.metadata(digest)?,
                "registry tag changed during publication"
            );
        }
        Ok(serde_json::json!({"artifact_digest":digest,"reference":value,"verified":true}))
    }
    fn push_blob(
        &self,
        client: &Client,
        image: &Reference,
        store: &Store,
        digest: &ArtifactDigest,
    ) -> Result<()> {
        for attempt in 0..3 {
            // A fresh verified FD makes every transfer attempt independently replayable.
            let file = store.open_blob(digest)?;
            let size = file.metadata()?.len();
            let stream = ReaderStream::with_capacity(tokio::fs::File::from_std(file), 65536)
                .map_err(|_| {
                    oci_client::errors::OciDistributionError::GenericError(Some(
                        "local blob read failed".into(),
                    ))
                });
            if remote(self.runtime.block_on(client.push_blob_stream(
                image,
                stream,
                digest.as_str(),
                Some(size),
            )))
            .is_ok()
            {
                // Do not trust an upload Location/header as the content receipt.
                let descriptor = oci_client::manifest::OciDescriptor {
                    digest: digest.as_str().to_owned(),
                    size: i64::try_from(size)?,
                    ..Default::default()
                };
                let stream = remote(
                    self.runtime
                        .block_on(client.pull_blob_stream(image, &descriptor)),
                )?;
                ensure!(
                    stream.content_length.is_none_or(|n| n == size),
                    "registry upload verification size mismatch"
                );
                let mut bridge = tokio_util::io::SyncIoBridge::new_with_handle(
                    tokio_util::io::StreamReader::new(stream.stream),
                    self.runtime.handle().clone(),
                );
                let (actual, received) = crate::cas::hash(&mut bridge, size)
                    .map_err(|_| anyhow::anyhow!("registry upload verification failed"))?;
                ensure!(
                    actual == *digest && received == size,
                    "registry upload verification mismatch"
                );
                return Ok(());
            }
            self.backoff(attempt);
        }
        anyhow::bail!("registry blob upload failed")
    }
    fn push_manifest(
        &self,
        client: &Client,
        image: &Reference,
        manifest: &Manifest,
        store: &Store,
        auth: &oci_client::secrets::RegistryAuth,
    ) -> Result<()> {
        let content_type = http::HeaderValue::from_str(&manifest.media_type)?;
        let expected = bytes::Bytes::from(store.metadata(&manifest.digest)?);
        for attempt in 0..3 {
            if remote(self.runtime.block_on(client.push_manifest_raw(
                image,
                expected.clone(),
                content_type.clone(),
            )))
            .is_ok()
            {
                // Verify the immutable reference, including after an optional tag write.
                let pinned = image.clone_with_digest(manifest.digest.as_str().to_owned());
                let (bytes, _) = remote(self.runtime.block_on(client.pull_manifest_raw(
                    &pinned,
                    auth,
                    &[&manifest.media_type],
                )))?;
                ensure!(
                    bytes == expected,
                    "registry manifest upload verification mismatch"
                );
                return Ok(());
            }
            self.backoff(attempt);
        }
        anyhow::bail!("registry manifest upload failed")
    }
}

fn collect(
    store: &Store,
    digest: &ArtifactDigest,
    seen: &mut BTreeSet<ArtifactDigest>,
    manifests: &mut Vec<Manifest>,
    blobs: &mut BTreeSet<ArtifactDigest>,
    depth: usize,
) -> Result<()> {
    ensure!(
        depth <= 8 && seen.len() < store.limits.max_graph,
        "registry push graph limit exceeded"
    );
    if !seen.insert(digest.clone()) {
        return Ok(());
    }
    let bytes = store.metadata(digest)?;
    let value: serde_json::Value = serde_json::from_slice(&bytes)?;
    let media_type = if value.get("manifests").is_some() {
        let index: ImageIndex = serde_json::from_slice(&bytes)?;
        for child in index.manifests() {
            ensure!(
                matches!(
                    child.media_type(),
                    MediaType::ImageIndex | MediaType::ImageManifest
                ),
                "invalid push index descriptor"
            );
            collect(
                store,
                &store.descriptor(child)?,
                seen,
                manifests,
                blobs,
                depth + 1,
            )?;
        }
        MediaType::ImageIndex.to_string()
    } else {
        let manifest: ImageManifest = serde_json::from_slice(&bytes)?;
        for child in std::iter::once(manifest.config()).chain(manifest.layers()) {
            add_blob(store, child, blobs)?;
        }
        MediaType::ImageManifest.to_string()
    };
    ensure!(
        seen.len() + blobs.len() <= store.limits.max_graph,
        "registry push graph limit exceeded"
    );
    manifests.push(Manifest {
        digest: digest.clone(),
        media_type,
    });
    Ok(())
}
fn add_blob(
    store: &Store,
    descriptor: &Descriptor,
    blobs: &mut BTreeSet<ArtifactDigest>,
) -> Result<()> {
    blobs.insert(store.descriptor(descriptor)?);
    Ok(())
}
