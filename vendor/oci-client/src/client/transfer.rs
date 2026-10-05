impl Client {
    /// Fetches tags for a repository reference.
    pub async fn list_tags(
        &self,
        image: &Reference,
        auth: &RegistryAuth,
        n: Option<usize>,
        last: Option<&str>,
    ) -> Result<TagResponse> {
        let op = RegistryOperation::Pull;
        let url = self.to_list_tags_url(image);

        self.store_auth_if_needed(image.resolve_registry(), auth)
            .await;

        let request = self.client.get(&url);
        let request = if let Some(num) = n {
            request.query(&[("n", num)])
        } else {
            request
        };
        let request = if let Some(l) = last {
            request.query(&[("last", l)])
        } else {
            request
        };
        let request = RequestBuilderWrapper {
            client: self,
            request_builder: request,
        };
        let res = request
            .apply_auth(image, op)
            .await?
            .into_request_builder()
            .send()
            .await?;
        let status = res.status();
        let body = read_bounded_response(res, self.config.max_response_bytes).await?;

        validate_registry_response(status, &body, &url)?;

        Ok(serde_json::from_str(std::str::from_utf8(&body)?)?)
    }

    /// Pull an image and return the bytes
    ///
    /// The client will check if it's already been authenticated and if
    /// not will attempt to do.
    pub async fn pull(
        &self,
        image: &Reference,
        auth: &RegistryAuth,
        accepted_media_types: Vec<&str>,
    ) -> Result<ImageData> {
        debug!("Pulling image: {:?}", image);
        self.store_auth_if_needed(image.resolve_registry(), auth)
            .await;

        let (manifest, digest, config) = self._pull_manifest_and_config(image).await?;

        self.validate_layers(&manifest, accepted_media_types)
            .await?;

        let layers = stream::iter(&manifest.layers)
            .map(|layer| {
                // This avoids moving `self` which is &Self
                // into the async block. We only want to capture
                // as &Self
                let this = &self;
                async move {
                    let mut out: Vec<u8> = Vec::new();
                    debug!("Pulling image layer");
                    this.pull_blob(image, layer, &mut out).await?;
                    Ok::<_, OciDistributionError>(ImageLayer::new(
                        out,
                        layer.media_type.clone(),
                        layer.annotations.clone(),
                    ))
                }
            })
            .boxed() // Workaround to rustc issue https://github.com/rust-lang/rust/issues/104382
            .buffer_unordered(self.config.max_concurrent_download)
            .try_collect()
            .await?;

        Ok(ImageData {
            layers,
            manifest: Some(manifest),
            config,
            digest: Some(digest),
        })
    }

    /// Checks if a blob exists in the remote registry
    pub async fn blob_exists(&self, image: &Reference, digest: &str) -> Result<bool> {
        let url = self.to_v2_blob_url(image, digest);
        let request = RequestBuilderWrapper {
            client: self,
            request_builder: self.client.head(&url),
        };

        let res = request
            .apply_auth(image, RegistryOperation::Pull)
            .await?
            .into_request_builder()
            .send()
            .await?;

        match res.error_for_status() {
            Ok(_) => Ok(true),
            Err(err) => {
                if err.status() == Some(StatusCode::NOT_FOUND) {
                    Ok(false)
                } else {
                    Err(err.into())
                }
            }
        }
    }

    /// Push an image and return the uploaded URL of the image
    ///
    /// The client will check if it's already been authenticated and if
    /// not will attempt to do.
    ///
    /// If a manifest is not provided, the client will attempt to generate
    /// it from the provided image and config data.
    ///
    /// Returns pullable URL for the image
    pub async fn push(
        &self,
        image_ref: &Reference,
        layers: &[ImageLayer],
        config: Config,
        auth: &RegistryAuth,
        manifest: Option<OciImageManifest>,
    ) -> Result<PushResponse> {
        debug!("Pushing image: {:?}", image_ref);
        self.store_auth_if_needed(image_ref.resolve_registry(), auth)
            .await;

        let manifest: OciImageManifest = match manifest {
            Some(m) => m,
            None => OciImageManifest::build(layers, &config, None),
        };

        // Upload layers.
        //
        // Reuse the per-layer digests already computed while building (or
        // supplied with) the manifest, rather than hashing every layer a
        // second time here. For large layers this avoids a full redundant
        // SHA-256 pass over the data. When `build` produced the manifest its
        // `layers` are in the same order as `layers`; if a caller supplied a
        // manifest whose layer count does not match, fall back to hashing each
        // layer so behaviour is unchanged.
        let layer_digests: Vec<String> = if manifest.layers.len() == layers.len() {
            manifest.layers.iter().map(|d| d.digest.clone()).collect()
        } else {
            layers.iter().map(|l| l.sha256_digest()).collect()
        };
        stream::iter(layers.iter().zip(layer_digests))
            .map(|(layer, digest)| {
                // This avoids moving `self` which is &Self
                // into the async block. We only want to capture
                // as &Self
                let this = &self;
                async move {
                    this.push_blob(image_ref, layer.data.clone(), &digest)
                        .await?;
                    Result::Ok(())
                }
            })
            .boxed() // Workaround to rustc issue https://github.com/rust-lang/rust/issues/104382
            .buffer_unordered(self.config.max_concurrent_upload)
            .try_for_each(future::ok)
            .await?;

        let config_url = self
            .push_blob(image_ref, config.data, &manifest.config.digest)
            .await?;
        let manifest_url = self.push_manifest(image_ref, &manifest.into()).await?;

        Ok(PushResponse {
            config_url,
            manifest_url,
        })
    }

    /// Pushes a blob to the registry
    pub async fn push_blob(
        &self,
        image_ref: &Reference,
        data: impl Into<bytes::Bytes>,
        digest: &str,
    ) -> Result<String> {
        if self.config.use_monolithic_push {
            return self.push_blob_monolithically(image_ref, data, digest).await;
        }
        let data = data.into();
        // Cloning the bytes here is cheap (e.g. doesn't allocate anything except some space for
        // some pointers). If any cloning happened, it is because the caller's passed data was not
        // already a `Bytes` type or static data.
        match self
            .push_blob_chunked(image_ref, data.clone(), digest)
            .await
        {
            Ok(url) => Ok(url),
            Err(OciDistributionError::SpecViolationError(violation)) => {
                warn!(?violation, "Registry is not respecting the OCI Distribution Specification when doing chunked push operations");
                warn!("Attempting monolithic push");
                self.push_blob_monolithically(image_ref, data, digest).await
            }
            Err(e) => Err(e),
        }
    }

    /// Pushes a blob to the registry as a monolith
    ///
    /// Returns the pullable location of the blob
    async fn push_blob_monolithically(
        &self,
        image: &Reference,
        blob_data: impl Into<bytes::Bytes>,
        blob_digest: &str,
    ) -> Result<String> {
        let location = self.begin_push_monolithical_session(image).await?;
        self.push_monolithically(&location, image, blob_data, blob_digest)
            .await
    }

    /// Pushes a blob to the registry as a series of chunks
    ///
    /// Returns the pullable location of the blob
    async fn push_blob_chunked(
        &self,
        image: &Reference,
        blob_data: impl Into<bytes::Bytes>,
        blob_digest: &str,
    ) -> Result<String> {
        let mut location = self.begin_push_chunked_session(image).await?;
        let mut start: usize = 0;

        let mut blob_data: bytes::Bytes = blob_data.into();
        while !blob_data.is_empty() {
            let chunk_size = self.push_chunk_size.min(blob_data.len());
            let chunk = blob_data.split_to(chunk_size);
            (location, start) = self.push_chunk(&location, image, chunk, start).await?;
        }
        self.end_push_chunked_session(&location, image, blob_digest)
            .await
    }

    /// Pushes a blob to the registry from an input stream.
    ///
    /// If `use_monolithic_push` is set in the client config, a single PUT is used (monolithic
    /// push). In that case `size` must be `Some`, as it is required to set `Content-Length` on the
    /// request. If `size` is `None` and `use_monolithic_push` is true, an error is returned.
    ///
    /// If `use_monolithic_push` is false the blob is sent as a series of chunked PATCH requests
    /// and `size` is ignored.
    ///
    /// Note: unlike [`push_blob`], there is no automatic fallback to monolithic push on a
    /// `SpecViolationError` from the chunked path, because a stream cannot be replayed after
    /// it has been consumed.
    ///
    /// Returns the pullable location of the blob.
    pub async fn push_blob_stream<T: Stream<Item = Result<bytes::Bytes>> + Send + 'static>(
        &self,
        image: &Reference,
        blob_data_stream: T,
        blob_digest: &str,
        size: Option<u64>,
    ) -> Result<String> {
        if self.config.use_monolithic_push {
            let size = size.ok_or_else(|| {
                OciDistributionError::GenericError(Some(
                    "size must be provided when use_monolithic_push is enabled".to_string(),
                ))
            })?;
            let location = self.begin_push_monolithical_session(image).await?;
            return self
                .push_stream_monolithically(&location, image, blob_data_stream, size, blob_digest)
                .await;
        }

        let location = self.begin_push_chunked_session(image).await?;
        let (location, _size) = self
            .push_stream_chunks(location, image, blob_data_stream, |_| {})
            .await?;
        self.end_push_chunked_session(&location, image, blob_digest)
            .await
    }

    /// Pushes a blob to the registry from an input stream using chunked transfer, computing
    /// the SHA256 digest on-the-fly.
    ///
    /// Unlike [`push_blob_stream`], the caller does not need to know the digest upfront.
    /// The digest is computed incrementally as each chunk is sent, then supplied to the
    /// registry in the final commit request.
    ///
    /// Monolithic push is not supported by this method; the blob is always sent as a series
    /// of chunked PATCH requests regardless of the `use_monolithic_push` setting.
    ///
    /// Returns the pullable location of the blob, the computed digest, and the blob size.
    pub async fn push_blob_stream_chunked<
        T: Stream<Item = Result<bytes::Bytes>> + Send + 'static,
    >(
        &self,
        image: &Reference,
        blob_data_stream: T,
    ) -> Result<PushBlobStreamChunkedResponse> {
        if self.config.use_monolithic_push {
            debug!(
                "use_monolithic_push is enabled, but push_blob_stream_chunked always uses chunked PATCH requests; ignoring"
            );
        }

        let location = self.begin_push_chunked_session(image).await?;

        let mut digester = Digester::Sha256(sha2::Sha256::new());
        let (location, size) = self
            .push_stream_chunks(location, image, blob_data_stream, |chunk| {
                digester.update(chunk)
            })
            .await?;

        if size == 0 {
            return Err(OciDistributionError::PushNoDataError);
        }

        let blob_digest = digester.finalize();
        let location = self
            .end_push_chunked_session(&location, image, &blob_digest)
            .await?;

        Ok(PushBlobStreamChunkedResponse {
            blob_url: location,
            blob_digest,
            size,
        })
    }

}
