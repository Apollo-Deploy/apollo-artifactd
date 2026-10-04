impl Client {
    /// Fetches the immutable digest for an image manifest.
    pub async fn fetch_manifest_digest(
        &self,
        image: &Reference,
        auth: &RegistryAuth,
    ) -> Result<String> {
        self.store_auth_if_needed(image.resolve_registry(), auth)
            .await;

        let url = self.to_v2_manifest_url(image);
        debug!("HEAD image manifest");
        let res = RequestBuilderWrapper::from_client(self, |client| client.head(&url))
            .apply_accept(MIME_TYPES_DISTRIBUTION_MANIFEST)?
            .apply_auth(image, RegistryOperation::Pull)
            .await?
            .into_request_builder()
            .send()
            .await?;

        if let Some(digest) = digest_header_value(res.headers().clone())? {
            let status = res.status();
            let body = read_bounded_response(res, self.config.max_response_bytes).await?;
            validate_registry_response(status, &body, &url)?;

            // If the reference has a digest and the digest header has a matching algorithm, compare
            // them and return an error if they don't match.
            if let Some(img_digest) = image.digest() {
                let header_digest = Digest::new(&digest)?;
                let image_digest = Digest::new(img_digest)?;
                if header_digest.algorithm == image_digest.algorithm
                    && header_digest != image_digest
                {
                    return Err(DigestError::VerificationError {
                        expected: img_digest.to_string(),
                        actual: digest,
                    }
                    .into());
                }
            }

            Ok(digest)
        } else {
            debug!("GET image manifest");
            let res = RequestBuilderWrapper::from_client(self, |client| client.get(&url))
                .apply_accept(MIME_TYPES_DISTRIBUTION_MANIFEST)?
                .apply_auth(image, RegistryOperation::Pull)
                .await?
                .into_request_builder()
                .send()
                .await?;
            let status = res.status();
            trace!(headers = ?res.headers(), "Got Headers");
            let headers = res.headers().clone();
            let body = read_bounded_response(res, self.config.max_response_bytes).await?;
            validate_registry_response(status, &body, &url)?;

            validate_digest(&body, digest_header_value(headers)?, image.digest())
                .map_err(OciDistributionError::from)
        }
    }

    async fn validate_layers(
        &self,
        manifest: &OciImageManifest,
        accepted_media_types: Vec<&str>,
    ) -> Result<()> {
        if manifest.layers.is_empty() {
            return Err(OciDistributionError::PullNoLayersError);
        }

        for layer in &manifest.layers {
            if !accepted_media_types.iter().any(|i| i.eq(&layer.media_type)) {
                return Err(OciDistributionError::IncompatibleLayerMediaTypeError(
                    layer.media_type.clone(),
                ));
            }
        }

        Ok(())
    }

    /// Pull a manifest from the remote OCI Distribution service.
    ///
    /// The client will check if it's already been authenticated and if
    /// not will attempt to do.
    ///
    /// A Tuple is returned containing the [OciImageManifest]
    /// and the manifest content digest hash.
    ///
    /// If a multi-platform Image Index manifest is encountered, a platform-specific
    /// Image manifest will be selected using the client's default platform resolution.
    pub async fn pull_image_manifest(
        &self,
        image: &Reference,
        auth: &RegistryAuth,
    ) -> Result<(OciImageManifest, String)> {
        self.store_auth_if_needed(image.resolve_registry(), auth)
            .await;

        self._pull_image_manifest(image).await
    }

    /// Pull a manifest from the remote OCI Distribution service.
    ///
    /// The client will check if it's already been authenticated and if
    /// not will attempt to do.
    ///
    /// Returns `(image_manifest, manifest_digest, Option<manifest_list_digest>)`.
    /// The manifest list digest is `Some` when the original reference pointed to
    /// an image index / manifest list; `None` when it pointed directly to a
    /// single-platform image manifest.
    ///
    /// If a multi-platform Image Index manifest is encountered, a platform-specific
    /// Image manifest will be selected using the client's default platform resolution.
    pub async fn pull_image_manifest_and_list_digest(
        &self,
        image: &Reference,
        auth: &RegistryAuth,
    ) -> Result<(OciImageManifest, String, Option<String>)> {
        self.store_auth_if_needed(image.resolve_registry(), auth)
            .await;

        self._pull_image_manifest_and_list_digest(image).await
    }

    /// Pull a manifest from the remote OCI Distribution service without parsing it.
    ///
    /// The client will check if it's already been authenticated and if
    /// not will attempt to do.
    ///
    /// A Tuple is returned containing raw byte representation of the manifest
    /// and the manifest content digest.
    pub async fn pull_manifest_raw(
        &self,
        image: &Reference,
        auth: &RegistryAuth,
        accepted_media_types: &[&str],
    ) -> Result<(bytes::Bytes, String)> {
        self.store_auth_if_needed(image.resolve_registry(), auth)
            .await;

        self._pull_manifest_raw(image, accepted_media_types).await
    }

    /// Pull a manifest from the remote OCI Distribution service.
    ///
    /// The client will check if it's already been authenticated and if
    /// not will attempt to do.
    ///
    /// A Tuple is returned containing the [Manifest](crate::manifest::OciImageManifest)
    /// and the manifest content digest hash.
    pub async fn pull_manifest(
        &self,
        image: &Reference,
        auth: &RegistryAuth,
    ) -> Result<(OciManifest, String)> {
        self.store_auth_if_needed(image.resolve_registry(), auth)
            .await;

        self._pull_manifest(image).await
    }

    /// Pull an image manifest from the remote OCI Distribution service.
    ///
    /// If the connection has already gone through authentication, this will
    /// use the bearer token. Otherwise, this will attempt an anonymous pull.
    ///
    /// If a multi-platform Image Index manifest is encountered, a platform-specific
    /// Image manifest will be selected using the client's default platform resolution.
    async fn _pull_image_manifest(&self, image: &Reference) -> Result<(OciImageManifest, String)> {
        let (manifest, digest, _list_digest) =
            self._pull_image_manifest_and_list_digest(image).await?;
        Ok((manifest, digest))
    }

    /// Pull an image manifest from the remote OCI Distribution service,
    /// also returning the manifest list digest if the image is multi-arch.
    ///
    /// If the connection has already gone through authentication, this will
    /// use the bearer token. Otherwise, this will attempt an anonymous pull.
    ///
    /// Returns `(image_manifest, manifest_digest, Option<manifest_list_digest>)`.
    /// The manifest list digest is `Some` when the original reference pointed to
    /// an image index / manifest list; `None` when it pointed directly to a
    /// single-platform image manifest.
    async fn _pull_image_manifest_and_list_digest(
        &self,
        image: &Reference,
    ) -> Result<(OciImageManifest, String, Option<String>)> {
        let (manifest, digest) = self._pull_manifest(image).await?;
        match manifest {
            OciManifest::Image(image_manifest) => Ok((image_manifest, digest, None)),
            OciManifest::ImageIndex(image_index_manifest) => {
                let list_digest = digest;
                debug!("Inspecting Image Index Manifest");
                let platform_digest = if let Some(resolver) = &self.config.platform_resolver {
                    resolver(&image_index_manifest.manifests)
                } else {
                    return Err(OciDistributionError::ImageIndexParsingNoPlatformResolverError);
                };

                match platform_digest {
                    Some(platform_digest) => {
                        debug!("Selected manifest entry with digest: {}", platform_digest);
                        let manifest_entry_reference =
                            image.clone_with_digest(platform_digest.clone());
                        self._pull_manifest(&manifest_entry_reference)
                            .await
                            .and_then(|(manifest, _digest)| match manifest {
                                OciManifest::Image(manifest) => {
                                    Ok((manifest, platform_digest, Some(list_digest)))
                                }
                                OciManifest::ImageIndex(_) => {
                                    Err(OciDistributionError::ImageManifestNotFoundError(
                                        "received Image Index manifest instead".to_string(),
                                    ))
                                }
                            })
                    }
                    None => Err(OciDistributionError::ImageManifestNotFoundError(
                        "no entry found in image index manifest matching client's default platform"
                            .to_string(),
                    )),
                }
            }
        }
    }

    /// Pull a manifest from the remote OCI Distribution service without parsing it.
    ///
    /// If the connection has already gone through authentication, this will
    /// use the bearer token. Otherwise, this will attempt an anonymous pull.
    async fn _pull_manifest_raw(
        &self,
        image: &Reference,
        accepted_media_types: &[&str],
    ) -> Result<(bytes::Bytes, String)> {
        let url = self.to_v2_manifest_url(image);
        debug!("Pulling image manifest");

        let res = RequestBuilderWrapper::from_client(self, |client| client.get(&url))
            .apply_accept(accepted_media_types)?
            .apply_auth(image, RegistryOperation::Pull)
            .await?
            .into_request_builder()
            .send()
            .await?;
        let status = res.status();
        let headers = res.headers().clone();
        let body = read_bounded_response(res, self.config.max_response_bytes).await?;

        validate_registry_response(status, &body, &url)?;

        let digest_header = digest_header_value(headers)?;
        let digest = validate_digest(&body, digest_header, image.digest())?;

        Ok((body, digest))
    }

    /// Pull a manifest from the remote OCI Distribution service.
    ///
    /// If the connection has already gone through authentication, this will
    /// use the bearer token. Otherwise, this will attempt an anonymous pull.
    async fn _pull_manifest(&self, image: &Reference) -> Result<(OciManifest, String)> {
        let (body, digest) = self
            ._pull_manifest_raw(image, MIME_TYPES_DISTRIBUTION_MANIFEST)
            .await?;

        self.validate_image_manifest(&body).await?;

        debug!("Parsing response as Manifest");
        let manifest = serde_json::from_slice(&body)
            .map_err(|e| OciDistributionError::ManifestParsingError(e.to_string()))?;
        Ok((manifest, digest))
    }

    async fn validate_image_manifest(&self, body: &[u8]) -> Result<()> {
        let versioned: Versioned = serde_json::from_slice(body)
            .map_err(|e| OciDistributionError::VersionedParsingError(e.to_string()))?;
        debug!(?versioned, "validating manifest");
        if versioned.schema_version != 2 {
            return Err(OciDistributionError::UnsupportedSchemaVersionError(
                versioned.schema_version,
            ));
        }
        if let Some(media_type) = versioned.media_type {
            if media_type != IMAGE_MANIFEST_MEDIA_TYPE
                && media_type != OCI_IMAGE_MEDIA_TYPE
                && media_type != IMAGE_MANIFEST_LIST_MEDIA_TYPE
                && media_type != OCI_IMAGE_INDEX_MEDIA_TYPE
            {
                return Err(OciDistributionError::UnsupportedMediaTypeError(media_type));
            }
        }

        Ok(())
    }

    /// Pull a manifest and its config from the remote OCI Distribution service.
    ///
    /// The client will check if it's already been authenticated and if
    /// not will attempt to do.
    ///
    /// A Tuple is returned containing the [OciImageManifest],
    /// the manifest content digest hash and the contents of the manifests config layer
    /// as a String.
    pub async fn pull_manifest_and_config(
        &self,
        image: &Reference,
        auth: &RegistryAuth,
    ) -> Result<(OciImageManifest, String, String)> {
        self.store_auth_if_needed(image.resolve_registry(), auth)
            .await;

        self._pull_manifest_and_config(image)
            .await
            .and_then(|(manifest, digest, config)| {
                Ok((
                    manifest,
                    digest,
                    String::from_utf8(config.data.into()).map_err(|e| {
                        OciDistributionError::GenericError(Some(format!(
                            "Cannot parse config as UTF-8 string: {e}"
                        )))
                    })?,
                ))
            })
    }

    /// Pull a manifest and its config from the remote OCI Distribution service.
    ///
    /// The client will check if it's already been authenticated and if
    /// not will attempt to do.
    ///
    /// Returns `(image_manifest, manifest_digest, config_json, Option<manifest_list_digest>)`.
    /// The manifest list digest is `Some` when the original reference pointed to
    /// an image index / manifest list; `None` when it pointed directly to a
    /// single-platform image manifest.
    ///
    /// If a multi-platform Image Index manifest is encountered, a platform-specific
    /// Image manifest will be selected using the client's default platform resolution.
    pub async fn pull_manifest_and_config_and_list_digest(
        &self,
        image: &Reference,
        auth: &RegistryAuth,
    ) -> Result<(OciImageManifest, String, String, Option<String>)> {
        self.store_auth_if_needed(image.resolve_registry(), auth)
            .await;

        self._pull_manifest_and_config_and_list_digest(image)
            .await
            .and_then(|(manifest, digest, config, list_digest)| {
                Ok((
                    manifest,
                    digest,
                    String::from_utf8(config.data.into()).map_err(|e| {
                        OciDistributionError::GenericError(Some(format!(
                            "Cannot parse config as UTF-8 string: {e}"
                        )))
                    })?,
                    list_digest,
                ))
            })
    }

    async fn _pull_manifest_and_config(
        &self,
        image: &Reference,
    ) -> Result<(OciImageManifest, String, Config)> {
        let (manifest, digest, config, _list_digest) = self
            ._pull_manifest_and_config_and_list_digest(image)
            .await?;
        Ok((manifest, digest, config))
    }

    async fn _pull_manifest_and_config_and_list_digest(
        &self,
        image: &Reference,
    ) -> Result<(OciImageManifest, String, Config, Option<String>)> {
        let (manifest, digest, list_digest) =
            self._pull_image_manifest_and_list_digest(image).await?;

        let mut out: Vec<u8> = Vec::new();
        debug!("Pulling config layer");
        self.pull_blob(image, &manifest.config, &mut out).await?;
        let media_type = manifest.config.media_type.clone();
        let annotations = manifest.annotations.clone();
        Ok((
            manifest,
            digest,
            Config::new(out, media_type, annotations),
            list_digest,
        ))
    }

}
