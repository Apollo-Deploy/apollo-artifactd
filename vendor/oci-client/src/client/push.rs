impl Client {
    /// Mounts an existing blob from another repository.
    pub async fn mount_blob(
        &self,
        image: &Reference,
        source: &Reference,
        digest: &str,
    ) -> Result<()> {
        let base_url = self.to_v2_blob_upload_url(image);
        let url = Url::parse_with_params(
            &base_url,
            &[("mount", digest), ("from", source.repository())],
        )
        .map_err(|e| OciDistributionError::UrlParseError(e.to_string()))?;

        let res = RequestBuilderWrapper::from_client(self, |client| client.post(url.clone()))
            .apply_auth(image, RegistryOperation::Push)
            .await?
            .into_request_builder()
            .send()
            .await?;

        self.extract_location_header(image, res, &reqwest::StatusCode::CREATED)
            .await?;

        Ok(())
    }

    /// Pushes the manifest for a specified image
    ///
    /// Returns pullable manifest URL
    pub async fn push_manifest(&self, image: &Reference, manifest: &OciManifest) -> Result<String> {
        let mut headers = HeaderMap::new();
        let content_type = manifest.content_type();
        headers.insert("Content-Type", content_type.parse().unwrap());

        // Serialize the manifest with a canonical json formatter, as described at
        // https://github.com/opencontainers/image-spec/blob/main/considerations.md#json
        let mut body = Vec::new();
        let mut ser = serde_json::Serializer::with_formatter(&mut body, CanonicalFormatter::new());
        manifest.serialize(&mut ser).unwrap();

        self.push_manifest_raw(image, body, manifest.content_type().parse().unwrap())
            .await
    }

    /// Pushes the manifest, provided as raw bytes, for a specified image
    ///
    /// Returns pullable manifest url
    pub async fn push_manifest_raw(
        &self,
        image: &Reference,
        body: impl Into<bytes::Bytes>,
        content_type: HeaderValue,
    ) -> Result<String> {
        let url = self.to_v2_manifest_url(image);
        debug!(?content_type, "push manifest");

        let mut headers = HeaderMap::new();
        headers.insert("Content-Type", content_type);

        let body = body.into();

        // Calculate the digest of the manifest, this is useful
        // if the remote registry is violating the OCI Distribution Specification.
        // See below for more details.
        let manifest_hash = sha256_digest(&body);

        let res = RequestBuilderWrapper::from_client(self, |client| client.put(url.clone()))
            .apply_auth(image, RegistryOperation::Push)
            .await?
            .into_request_builder()
            .headers(headers)
            .body(body)
            .send()
            .await?;

        let ret = self
            .extract_location_header(image, res, &reqwest::StatusCode::CREATED)
            .await;

        if matches!(ret, Err(OciDistributionError::RegistryNoLocationError)) {
            // The registry is violating the OCI Distribution Spec, BUT the OCI
            // image/artifact has been uploaded successfully.
            // The `Location` header contains the sha256 digest of the manifest,
            // we can reuse the value we calculated before.
            // The workaround is there because repositories such as
            // AWS ECR are violating this aspect of the spec. This at least let the
            // oci-distribution users interact with these registries.
            warn!("Registry is not respecting the OCI Distribution Specification: it didn't return the Location of the uploaded Manifest inside of the response headers. Working around this issue...");

            let reference_suffix = image
                .digest()
                .or_else(|| image.tag())
                .unwrap_or("latest");
            let Some(url_base) = url.strip_suffix(reference_suffix) else {
                return Err(OciDistributionError::SpecViolationError(
                    "manifest upload URL did not contain the requested reference".to_string(),
                ));
            };
            let url_by_digest = format!("{url_base}{manifest_hash}");

            return Ok(url_by_digest);
        }

        ret
    }

    /// Pulls the referrers for the given image filtering by the optionally provided artifact type.
    ///
    /// Implements the [OCI Distribution Spec referrers API][oci-referrers] with an automatic
    /// fallback to the [referrers tag schema][oci-tag-schema] when the registry returns a
    /// `404 Not Found` for the native endpoint (as required by the spec).
    ///
    /// Many registries (e.g. ghcr.io) do not implement the native
    /// `/v2/<name>/referrers/<digest>` endpoint and return 404 instead. The OCI spec
    /// defines a fallback: the referrers index is stored as a regular OCI Image Index
    /// under a tag derived from the subject digest by replacing `:` with `-`
    /// (e.g. `sha256:abc…` → tag `sha256-abc…`).
    ///
    /// When the fallback is used, `artifact_type` filtering is applied client-side,
    /// since the tag schema stores a single unfiltered index with no query-parameter
    /// support.
    ///
    /// If both the native API and the tag schema fail, an empty `OciImageIndex` is
    /// returned, as per the spec recommendation.
    ///
    /// [oci-referrers]: https://github.com/opencontainers/distribution-spec/blob/main/spec.md#listing-referrers
    /// [oci-tag-schema]: https://github.com/opencontainers/distribution-spec/blob/main/spec.md#referrers-tag-schema
    pub async fn pull_referrers(
        &self,
        image: &Reference,
        artifact_type: Option<&str>,
    ) -> Result<OciImageIndex> {
        let url = self.to_v2_referrers_url(image, artifact_type)?;

        let res = RequestBuilderWrapper::from_client(self, |client| client.get(&url))
            .apply_accept(MIME_TYPES_DISTRIBUTION_MANIFEST)?
            .apply_auth(image, RegistryOperation::Pull)
            .await?
            .into_request_builder()
            .send()
            .await?;
        let status = res.status();
        let body = read_bounded_response(res, self.config.max_response_bytes).await?;

        // Per the OCI Distribution Spec, a 404 on the native referrers endpoint means the
        // registry does not support it; fall back to the referrers tag schema.
        if status == reqwest::StatusCode::NOT_FOUND {
            debug!("Native referrers API returned 404; falling back to OCI referrers tag schema");
            return self
                .pull_referrers_via_tag_schema(image, artifact_type)
                .await;
        }

        validate_registry_response(status, &body, &url)?;
        let manifest = serde_json::from_slice(&body)
            .map_err(|e| OciDistributionError::ManifestParsingError(e.to_string()))?;

        Ok(manifest)
    }

    /// Pulls the referrers index using the OCI referrers tag schema fallback.
    ///
    /// The tag is the subject digest with `:` replaced by `-`
    /// (e.g. `sha256:abc…` → `sha256-abc…`).
    ///
    /// If `artifact_type` is provided, the returned index is filtered client-side
    /// to include only entries whose `artifact_type` matches.
    ///
    /// If the tag does not exist or does not contain a valid image index, an empty
    /// `OciImageIndex` is returned as per the OCI spec recommendation.
    async fn pull_referrers_via_tag_schema(
        &self,
        image: &Reference,
        artifact_type: Option<&str>,
    ) -> Result<OciImageIndex> {
        let digest = image.digest().ok_or_else(|| {
            OciDistributionError::GenericError(Some(
                "Getting referrers for a tag is not supported".into(),
            ))
        })?;

        let fallback_tag = digest.replace(':', "-");
        let fallback_ref = Reference::with_tag(
            image.resolve_registry().to_string(),
            image.repository().to_string(),
            fallback_tag.clone(),
        );

        debug!(
            tag = %fallback_tag,
            "Pulling referrers via tag schema"
        );

        let manifest = match self._pull_manifest(&fallback_ref).await {
            Ok((manifest, _digest)) => manifest,
            Err(e) => match &e {
                OciDistributionError::ImageManifestNotFoundError(_)
                | OciDistributionError::RegistryError { .. }
                | OciDistributionError::ServerError { code: 404, .. } => {
                    debug!(
                        error = ?e,
                        "Referrers tag schema not found; assuming no referrers"
                    );
                    return Ok(empty_image_index());
                }
                _ => return Err(e),
            },
        };

        let mut index = match manifest {
            OciManifest::ImageIndex(idx) => idx,
            OciManifest::Image(_) => {
                return Err(OciDistributionError::SpecViolationError(format!(
                    "referrers tag schema: tag '{fallback_tag}' contains an Image manifest; \
                     expected an OCI Image Index"
                )));
            }
        };

        // Apply client-side artifact_type filtering when requested, since the tag
        // schema stores a single unfiltered index.
        if let Some(at) = artifact_type {
            index.manifests.retain(|entry| {
                entry
                    .artifact_type
                    .as_deref()
                    .map(|t| t == at)
                    .unwrap_or(false)
            });
        }

        Ok(index)
    }

    /// Lists available repositories in the registry.
    ///
    /// Implements the OCI Distribution Spec catalog endpoint (`/v2/_catalog`).
    /// Supports pagination via `n` (page size) and `last` (last repo from
    /// previous page).
    pub async fn catalog(
        &self,
        image: &Reference,
        auth: &RegistryAuth,
        n: Option<usize>,
        last: Option<&str>,
    ) -> Result<CatalogResponse> {
        let op = RegistryOperation::Pull;
        let url = self.to_catalog_url(image);

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

    async fn extract_location_header(
        &self,
        image: &Reference,
        res: reqwest::Response,
        expected_status: &reqwest::StatusCode,
    ) -> Result<String> {
        debug!(expected_status_code=?expected_status.as_u16(),
            status_code=?res.status().as_u16(),
            "extract location header");
        if res.status().eq(expected_status) {
            let location_header = res.headers().get("Location");
            debug!("Received upload location header");
            match location_header {
                None => Err(OciDistributionError::RegistryNoLocationError),
                Some(lh) => self.location_header_to_url(image, lh),
            }
        } else if res.status().is_success() && expected_status.is_success() {
            Err(OciDistributionError::SpecViolationError(format!(
                "Expected HTTP Status {}, got {} instead",
                expected_status,
                res.status(),
            )))
        } else {
            let url = res.url().to_string();
            let code = res.status().as_u16();
            let message = String::from_utf8_lossy(
                &read_bounded_response(res, self.config.max_response_bytes).await?,
            )
            .to_string();
            Err(OciDistributionError::ServerError { url, code, message })
        }
    }

    /// Helper function to convert location header to URL
    ///
    /// Location may be absolute (containing the protocol and/or hostname), or relative (containing just the URL path)
    /// Returns a properly formatted absolute URL
    ///
    /// An absolute location pointing at a different host is returned as is and
    /// will be followed, but requests to it are never authenticated (see
    /// [`RequestBuilderWrapper::apply_auth`]).
    fn location_header_to_url(
        &self,
        image: &Reference,
        location_header: &reqwest::header::HeaderValue,
    ) -> Result<String> {
        let lh = location_header.to_str()?;
        if lh.starts_with("/") {
            let registry = image.resolve_registry();
            Ok(format!(
                "{scheme}://{registry}{lh}",
                scheme = self.config.protocol.scheme_for(registry)
            ))
        } else {
            let location = Url::parse(lh).map_err(|_| {
                OciDistributionError::SpecViolationError(
                    "registry upload Location is not a valid URL".to_string(),
                )
            })?;
            if !location.username().is_empty() || location.password().is_some() {
                return Err(OciDistributionError::SpecViolationError(
                    "registry upload Location must not contain userinfo".to_string(),
                ));
            }
            let registry = image.resolve_registry();
            let expected_scheme = self.config.protocol.scheme_for(registry);
            if expected_scheme == "https" && location.scheme() != "https" {
                return Err(OciDistributionError::SpecViolationError(
                    "secure registry upload Location must use HTTPS".to_string(),
                ));
            }
            if location.scheme() != "http" && location.scheme() != "https" {
                return Err(OciDistributionError::SpecViolationError(
                    "registry upload Location must use HTTP or HTTPS".to_string(),
                ));
            }
            Ok(location.to_string())
        }
    }

    /// Convert a Reference to a v2 manifest URL.
    fn to_v2_manifest_url(&self, reference: &Reference) -> String {
        let registry = reference.resolve_registry();
        format!(
            "{scheme}://{registry}/v2/{repository}/manifests/{reference}{ns}",
            scheme = self.config.protocol.scheme_for(registry),
            repository = reference.repository(),
            reference = if let Some(digest) = reference.digest() {
                digest
            } else {
                reference.tag().unwrap_or("latest")
            },
            ns = reference
                .namespace()
                .map(|ns| format!("?ns={ns}"))
                .unwrap_or_default(),
        )
    }

    /// Convert a Reference to a v2 blob (layer) URL.
    fn to_v2_blob_url(&self, reference: &Reference, digest: &str) -> String {
        let registry = reference.resolve_registry();
        format!(
            "{scheme}://{registry}/v2/{repository}/blobs/{digest}{ns}",
            scheme = self.config.protocol.scheme_for(registry),
            repository = reference.repository(),
            ns = reference
                .namespace()
                .map(|ns| format!("?ns={ns}"))
                .unwrap_or_default(),
        )
    }

    /// Convert a Reference to a v2 blob upload URL.
    fn to_v2_blob_upload_url(&self, reference: &Reference) -> String {
        self.to_v2_blob_url(reference, "uploads/")
    }

    fn to_list_tags_url(&self, reference: &Reference) -> String {
        let registry = reference.resolve_registry();
        format!(
            "{scheme}://{registry}/v2/{repository}/tags/list{ns}",
            scheme = self.config.protocol.scheme_for(registry),
            repository = reference.repository(),
            ns = reference
                .namespace()
                .map(|ns| format!("?ns={ns}"))
                .unwrap_or_default(),
        )
    }

    fn to_catalog_url(&self, reference: &Reference) -> String {
        let registry = reference.resolve_registry();
        format!(
            "{scheme}://{registry}/v2/_catalog",
            scheme = self.config.protocol.scheme_for(registry),
        )
    }

    /// Convert a Reference to a v2 referrers URL.
    fn to_v2_referrers_url(
        &self,
        reference: &Reference,
        artifact_type: Option<&str>,
    ) -> Result<String> {
        let digest = reference.digest().ok_or_else(|| {
            OciDistributionError::GenericError(Some(
                "Getting referrers for a tag is not supported".into(),
            ))
        })?;

        let registry = reference.resolve_registry();
        let base = format!(
            "{scheme}://{registry}",
            scheme = self.config.protocol.scheme_for(registry),
        );
        let mut url =
            Url::parse(&base).map_err(|e| OciDistributionError::UrlParseError(e.to_string()))?;
        url.path_segments_mut()
            .map_err(|_| {
                OciDistributionError::GenericError(Some(
                    "cannot build referrers URL: base URL is cannot-be-a-base".into(),
                ))
            })?
            .push("v2")
            .extend(reference.repository().split('/'))
            .push("referrers")
            .push(digest);
        if let Some(at) = artifact_type {
            url.query_pairs_mut().append_pair("artifactType", at);
        }
        Ok(url.into())
}
    }
