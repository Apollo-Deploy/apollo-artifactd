impl Client {
    /// Pushes an OCI image index manifest.
    pub async fn push_manifest_list(
        &self,
        reference: &Reference,
        auth: &RegistryAuth,
        manifest: OciImageIndex,
    ) -> Result<String> {
        self.store_auth_if_needed(reference.resolve_registry(), auth)
            .await;
        self.push_manifest(reference, &OciManifest::ImageIndex(manifest))
            .await
    }

    /// Pull a single layer from an OCI registry.
    ///
    /// This pulls the layer for a particular image that is identified by the given layer
    /// descriptor. The layer descriptor can be anything that can be referenced as a layer
    /// descriptor. The image reference is used to find the repository and the registry, but it is
    /// not used to verify that the digest is a layer inside of the image. (The manifest is used for
    /// that.)
    pub async fn pull_blob<T: AsyncWrite>(
        &self,
        image: &Reference,
        layer: impl AsLayerDescriptor,
        out: T,
    ) -> Result<()> {
        let response = self.pull_blob_response(image, &layer, None, None).await?;

        let mut maybe_header_digester = digest_header_value(response.headers().clone())?
            .map(|digest| Digester::new(&digest).map(|d| (d, digest)))
            .transpose()?;

        // With a blob pull, we need to use the digest from the layer and not the image
        let layer_digest = layer.as_layer_descriptor().digest.to_string();
        let mut layer_digester = Digester::new(&layer_digest)?;

        let status = response.status();
        let url = response.url().to_string();
        if !status.is_success() {
            let body = read_bounded_response(response, self.config.max_response_bytes).await?;
            return validate_registry_response(status, &body, &url);
        }
        let mut stream = response.bytes_stream();

        let mut out = pin!(out);

        while let Some(bytes) = stream.next().await {
            let bytes = bytes?;
            if let Some((ref mut digester, _)) = maybe_header_digester.as_mut() {
                digester.update(&bytes);
            }
            layer_digester.update(&bytes);
            out.write_all(&bytes).await?;
        }

        // Ensure all buffered writes are flushed before returning.
        out.flush().await?;

        if let Some((mut digester, expected)) = maybe_header_digester.take() {
            let digest = digester.finalize();

            if digest != expected {
                return Err(DigestError::VerificationError {
                    expected,
                    actual: digest,
                }
                .into());
            }
        }

        let digest = layer_digester.finalize();
        if digest != layer_digest {
            return Err(DigestError::VerificationError {
                expected: layer_digest,
                actual: digest,
            }
            .into());
        }

        Ok(())
    }

    /// Stream a single layer from an OCI registry.
    ///
    /// This is a streaming version of [`Client::pull_blob`]. Returns [`SizedStream`], which
    /// implements [`Stream`] or can be used directly to get the content
    /// length of the response
    ///
    /// # Example
    /// ```rust
    /// use std::future::Future;
    /// use std::io::Error;
    ///
    /// use futures_util::TryStreamExt;
    /// use oci_client::{Client, Reference};
    /// use oci_client::client::ClientConfig;
    /// use oci_client::manifest::OciDescriptor;
    ///
    /// async {
    ///   let client = Client::new(Default::default());
    ///   let imgRef: Reference = "busybox:latest".parse().unwrap();
    ///   let desc = OciDescriptor { digest: "sha256:deadbeef".to_owned(), ..Default::default() };
    ///   let mut stream = client.pull_blob_stream(&imgRef, &desc).await.unwrap();
    ///   // Check the optional content length
    ///   let content_length = stream.content_length.unwrap_or_default();
    ///   // Use as a stream
    ///   stream.try_next().await.unwrap().unwrap();
    ///   // Use the underlying stream
    ///   let mut stream = stream.stream;
    /// };
    /// ```
    pub async fn pull_blob_stream(
        &self,
        image: &Reference,
        layer: impl AsLayerDescriptor,
    ) -> Result<SizedStream> {
        stream_from_response(
            self.pull_blob_response(image, &layer, None, None).await?,
            layer,
            true,
            self.config.max_response_bytes,
        )
        .await
    }

    /// Stream a single layer from an OCI registry starting with a byte offset. This can be used to
    /// continue downloading a layer after a network error. Please note that when doing a partial
    /// download (meaning it returns the [`BlobResponse::Partial`] variant), the layer digest is not
    /// verified as all the bytes are not available. The returned blob response will contain the
    /// header from the request digest, if it was set, that can be used (in addition to the digest
    /// from the layer) to verify the blob once all the bytes have been downloaded. Failure to do
    /// this means your content will not be verified.
    ///
    /// Returns [`BlobResponse`] which indicates if the response was a full or partial response.
    pub async fn pull_blob_stream_partial(
        &self,
        image: &Reference,
        layer: impl AsLayerDescriptor,
        offset: u64,
        length: Option<u64>,
    ) -> Result<BlobResponse> {
        let response = self
            .pull_blob_response(image, &layer, Some(offset), length)
            .await?;

        let status = response.status();
        match status {
            StatusCode::OK => Ok(BlobResponse::Full(
                stream_from_response(response, &layer, true, self.config.max_response_bytes).await?,
            )),
            StatusCode::PARTIAL_CONTENT => Ok(BlobResponse::Partial(
                stream_from_response(response, &layer, false, self.config.max_response_bytes).await?,
            )),
            _ => {
                let url = response.url().to_string();
                let body = read_bounded_response(response, self.config.max_response_bytes).await?;
                Err(validate_registry_response(status, &body, &url).expect_err("validate_registry_response should return an error for non-success status codes"))
            }
        }
    }

    /// Pull a single layer from an OCI registry.
    async fn pull_blob_response(
        &self,
        image: &Reference,
        layer: impl AsLayerDescriptor,
        offset: Option<u64>,
        length: Option<u64>,
    ) -> Result<Response> {
        let layer = layer.as_layer_descriptor();
        let url = self.to_v2_blob_url(image, layer.digest);

        let mut request = RequestBuilderWrapper::from_client(self, |client| client.get(&url))
            .apply_accept(MIME_TYPES_DISTRIBUTION_MANIFEST)?
            .apply_auth(image, RegistryOperation::Pull)
            .await?
            .into_request_builder();
        if let (Some(off), Some(len)) = (offset, length) {
            let end = (off + len).saturating_sub(1);
            request = request.header(
                RANGE,
                HeaderValue::from_str(&format!("bytes={off}-{end}")).unwrap(),
            );
        } else if let Some(offset) = offset {
            request = request.header(
                RANGE,
                HeaderValue::from_str(&format!("bytes={offset}-")).unwrap(),
            );
        }
        let mut response = request.send().await?;

        if let Some(urls) = &layer.urls {
            for url in urls {
                if response.error_for_status_ref().is_ok() {
                    break;
                }

                let url = Url::parse(url)
                    .map_err(|e| OciDistributionError::UrlParseError(e.to_string()))?;

                if url.scheme() == "http" || url.scheme() == "https" {
                    // NOTE: we must not authenticate on additional URLs as those
                    // can be abused to leak credentials or tokens.  Please
                    // refer to CVE-2020-15157 for more information.
                    request =
                        RequestBuilderWrapper::from_client(self, |client| client.get(url.clone()))
                            .apply_accept(MIME_TYPES_DISTRIBUTION_MANIFEST)?
                            .into_request_builder();
                    if let Some(offset) = offset {
                        request = request.header(
                            RANGE,
                            HeaderValue::from_str(&format!("bytes={offset}-")).unwrap(),
                        );
                    }
                    response = request.send().await?
                }
            }
        }

        Ok(response)
    }

    /// Begins a session to push an image to registry in a monolithical way
    ///
    /// Returns URL with session UUID
    async fn begin_push_monolithical_session(&self, image: &Reference) -> Result<String> {
        let url = &self.to_v2_blob_upload_url(image);
        let res = RequestBuilderWrapper::from_client(self, |client| client.post(url))
            .apply_auth(image, RegistryOperation::Push)
            .await?
            .into_request_builder()
            // We set "Content-Length" to 0 here even though the OCI Distribution
            // spec does not strictly require that. In practice we have seen that
            // certain registries require "Content-Length" to be present for all
            // types of push sessions.
            .header("Content-Length", 0)
            .send()
            .await?;

        // OCI spec requires the status code be 202 Accepted to successfully begin the push process
        self.extract_location_header(image, res, &reqwest::StatusCode::ACCEPTED)
            .await
    }

    /// Begins a session to push an image to registry as a series of chunks
    ///
    /// Returns URL with session UUID
    async fn begin_push_chunked_session(&self, image: &Reference) -> Result<String> {
        let url = &self.to_v2_blob_upload_url(image);
        let res = RequestBuilderWrapper::from_client(self, |client| client.post(url))
            .apply_auth(image, RegistryOperation::Push)
            .await?
            .into_request_builder()
            .header("Content-Length", 0)
            .send()
            .await?;

        // OCI spec requires the status code be 202 Accepted to successfully begin the push process
        self.extract_location_header(image, res, &reqwest::StatusCode::ACCEPTED)
            .await
    }

    /// Closes the chunked push session
    ///
    /// Returns the pullable URL for the image
    async fn end_push_chunked_session(
        &self,
        location: &str,
        image: &Reference,
        digest: &str,
    ) -> Result<String> {
        let url = Url::parse_with_params(location, &[("digest", digest)])
            .map_err(|e| OciDistributionError::GenericError(Some(e.to_string())))?;
        let res = RequestBuilderWrapper::from_client(self, |client| client.put(url.clone()))
            .apply_auth(image, RegistryOperation::Push)
            .await?
            .into_request_builder()
            .header("Content-Length", 0)
            .send()
            .await?;
        self.extract_location_header(image, res, &reqwest::StatusCode::CREATED)
            .await
    }

    /// Pushes a layer to a registry as a monolithical blob.
    ///
    /// Returns the URL location for the next layer
    async fn push_stream_monolithically(
        &self,
        location: &str,
        image: &Reference,
        layer: impl Stream<Item = Result<bytes::Bytes>> + Send + 'static,
        size: u64,
        blob_digest: &str,
    ) -> Result<String> {
        let mut url =
            Url::parse(location).map_err(|e| OciDistributionError::UrlParseError(e.to_string()))?;
        url.query_pairs_mut().append_pair("digest", blob_digest);
        let url = url.to_string();

        debug!(size, "Pushing monolithically");
        let mut headers = HeaderMap::new();
        headers.insert(
            "Content-Length",
            format!("{}", size)
                .parse()
                .map_err(|e: reqwest::header::InvalidHeaderValue| {
                    OciDistributionError::GenericError(Some(e.to_string()))
                })?,
        );
        headers.insert("Content-Type", "application/octet-stream".parse().unwrap());

        let res = RequestBuilderWrapper::from_client(self, |client| client.put(&url))
            .apply_auth(image, RegistryOperation::Push)
            .await?
            .into_request_builder()
            .headers(headers)
            .body(reqwest::Body::wrap_stream(layer))
            .send()
            .await?;

        // Returns location
        self.extract_location_header(image, res, &reqwest::StatusCode::CREATED)
            .await
    }

    /// Pushes a layer to a registry as a monolithical blob.
    ///
    /// Returns the URL location for the next layer
    async fn push_monolithically(
        &self,
        location: &str,
        image: &Reference,
        layer: impl Into<bytes::Bytes>,
        blob_digest: &str,
    ) -> Result<String> {
        let mut url = Url::parse(location).unwrap();
        url.query_pairs_mut().append_pair("digest", blob_digest);
        let url = url.to_string();

        let layer = layer.into();
        debug!(size = layer.len(), "Pushing monolithically");
        if layer.is_empty() {
            return Err(OciDistributionError::PushNoDataError);
        };
        let mut headers = HeaderMap::new();
        headers.insert(
            "Content-Length",
            format!("{}", layer.len()).parse().unwrap(),
        );
        headers.insert("Content-Type", "application/octet-stream".parse().unwrap());

        let res = RequestBuilderWrapper::from_client(self, |client| client.put(&url))
            .apply_auth(image, RegistryOperation::Push)
            .await?
            .into_request_builder()
            .headers(headers)
            .body(layer)
            .send()
            .await?;

        // Returns location
        self.extract_location_header(image, res, &reqwest::StatusCode::CREATED)
            .await
    }

    /// Pushes a single chunk of a blob to a registry, as part of a chunked blob upload.
    /// The caller is responsible for chunking the blob data into smaller parts, if needed.
    ///
    /// Returns the URL location for the next chunk, alongside the start of the next range to upload.
    async fn push_chunk(
        &self,
        location: &str,
        image: &Reference,
        blob_chunk: bytes::Bytes,
        range_start: usize,
    ) -> Result<(String, usize)> {
        if blob_chunk.is_empty() {
            return Err(OciDistributionError::PushNoDataError);
        };

        let chunk_size = blob_chunk.len();
        let end_range_inclusive = range_start + chunk_size - 1;

        let mut headers = HeaderMap::new();
        headers.insert(
            "Content-Range",
            format!("{range_start}-{end_range_inclusive}")
                .parse()
                .unwrap(),
        );

        headers.insert("Content-Length", format!("{chunk_size}").parse().unwrap());
        headers.insert("Content-Type", "application/octet-stream".parse().unwrap());

        debug!(
            ?range_start,
            ?end_range_inclusive,
            chunk_size,
            ?location,
            ?headers,
            "Pushing chunk"
        );

        let res = RequestBuilderWrapper::from_client(self, |client| client.patch(location))
            .apply_auth(image, RegistryOperation::Push)
            .await?
            .into_request_builder()
            .headers(headers)
            .body(blob_chunk)
            .send()
            .await?;

        // Returns location for next chunk and the start byte for the next range
        Ok((
            self.extract_location_header(image, res, &reqwest::StatusCode::ACCEPTED)
                .await?,
            end_range_inclusive + 1,
        ))
    }

    /// Sends `stream` to an already-open chunked upload session as a series of PATCH requests.
    ///
    /// `on_chunk` is invoked with each chunk right before it is sent, allowing callers to
    /// incrementally compute a digest or otherwise observe the data without buffering it.
    ///
    /// Returns the upload location to use for the final commit request, alongside the total
    /// number of bytes sent.
    async fn push_stream_chunks(
        &self,
        location: String,
        image: &Reference,
        blob_data_stream: impl Stream<Item = Result<bytes::Bytes>> + Send + 'static,
        mut on_chunk: impl FnMut(&bytes::Bytes),
    ) -> Result<(String, u64)> {
        let mut location = location;
        let mut range_start = 0;
        let mut size = 0u64;

        let mut blob_data_stream = pin!(blob_data_stream);

        while let Some(blob_data) = blob_data_stream.next().await {
            let mut blob_data = blob_data?;
            while !blob_data.is_empty() {
                let chunk = blob_data.split_to(self.push_chunk_size.min(blob_data.len()));
                size += chunk.len() as u64;
                on_chunk(&chunk);
                (location, range_start) = self
                    .push_chunk(&location, image, chunk, range_start)
                    .await?;
            }
        }

        Ok((location, size))
    }

}
