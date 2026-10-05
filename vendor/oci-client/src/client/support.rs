/// The OCI spec technically does not allow any codes but 200, 500, 401, and 404.
/// Obviously, HTTP servers are going to send other codes. This tries to catch the
/// obvious ones (200, 4XX, 5XX). Anything else is just treated as an error.
fn validate_registry_response(status: reqwest::StatusCode, body: &[u8], url: &str) -> Result<()> {
    match status {
        reqwest::StatusCode::OK => Ok(()),
        reqwest::StatusCode::UNAUTHORIZED => Err(OciDistributionError::UnauthorizedError {
            url: url.to_string(),
        }),
        s if s.is_success() => Err(OciDistributionError::SpecViolationError(format!(
            "Expected HTTP Status {}, got {} instead",
            reqwest::StatusCode::OK,
            status,
        ))),
        s if s.is_client_error() => {
            match serde_json::from_slice::<OciEnvelope>(body) {
                // According to the OCI spec, we should see an error in the message body.
                Ok(envelope) => Err(OciDistributionError::RegistryError {
                    envelope,
                    url: url.to_string(),
                }),
                // Fall back to a plain server error if the body isn't a valid `OciEnvelope`
                Err(_) => Err(OciDistributionError::ServerError {
                    code: s.as_u16(),
                    url: url.to_string(),
                    message: String::from_utf8_lossy(body).to_string(),
                }),
            }
        }
        // Catch-all for any remaining status: mostly 5xx, but also 1xx, 3xx and non-standard codes.
        // Use a lossy conversion so a non UTF-8 body doesn't hide the status code
        s => Err(OciDistributionError::ServerError {
            code: s.as_u16(),
            url: url.to_string(),
            message: String::from_utf8_lossy(body).to_string(),
        }),
    }
}

/// Returns an empty OCI Image Index, as used when no referrers exist.
fn empty_image_index() -> OciImageIndex {
    OciImageIndex {
        schema_version: 2,
        media_type: Some(crate::manifest::OCI_IMAGE_INDEX_MEDIA_TYPE.to_string()),
        artifact_type: None,
        annotations: None,
        manifests: vec![],
    }
}

/// Converts a response into a stream
async fn stream_from_response(
    response: Response,
    layer: impl AsLayerDescriptor,
    verify: bool,
    max_response_bytes: usize,
) -> Result<SizedStream> {
    let status = response.status();
    let url = response.url().to_string();
    let content_length = response.content_length();
    let headers = response.headers().clone();
    if !status.is_success() {
        let body = read_bounded_response(response, max_response_bytes).await?;
        return Err(validate_registry_response(status, &body, &url).expect_err(
            "validate_registry_response should return an error for non-success status codes",
        ));
    }
    let stream = response.bytes_stream().map_err(std::io::Error::other);

    let expected_layer_digest = layer.as_layer_descriptor().digest.to_string();
    let layer_digester = Digester::new(&expected_layer_digest)?;
    let header_digester_and_digest = match digest_header_value(headers)? {
        // If the digests match, we don't need to do both digesters
        Some(digest) if digest == expected_layer_digest => None,
        Some(digest) => Some((Digester::new(&digest)?, digest)),
        None => None,
    };
    let header_digest = header_digester_and_digest
        .as_ref()
        .map(|(_, digest)| digest.to_owned());
    let stream: BoxStream<'static, std::result::Result<bytes::Bytes, std::io::Error>> = if verify {
        Box::pin(VerifyingStream::new(
            Box::pin(stream),
            layer_digester,
            expected_layer_digest,
            header_digester_and_digest,
        ))
    } else {
        Box::pin(stream)
    };
    Ok(SizedStream {
        content_length,
        digest_header_value: header_digest,
        stream,
    })
}

/// The request builder wrapper allows to be instantiated from a
/// `Client` and allows composable operations on the request builder,
/// to produce a `RequestBuilder` object that can be executed.
struct RequestBuilderWrapper<'a> {
    client: &'a Client,
    request_builder: RequestBuilder,
}

// RequestBuilderWrapper type management
impl<'a> RequestBuilderWrapper<'a> {
    /// Create a `RequestBuilderWrapper` from a `Client` instance, by
    /// instantiating the internal `RequestBuilder` with the provided
    /// function `f`.
    fn from_client(
        client: &'a Client,
        f: impl Fn(&reqwest::Client) -> RequestBuilder,
    ) -> RequestBuilderWrapper<'a> {
        let request_builder = f(&client.client);
        RequestBuilderWrapper {
            client,
            request_builder,
        }
    }

    // Produces a final `RequestBuilder` out of this `RequestBuilderWrapper`
    fn into_request_builder(self) -> RequestBuilder {
        self.request_builder
    }
}

// Composable functions applicable to a `RequestBuilderWrapper`
impl<'a> RequestBuilderWrapper<'a> {
    /// Returns a clone of the inner `RequestBuilder`.
    ///
    /// Cloning fails if the request has a streaming body, which is never the
    /// case here since bodies are attached after the wrapper is consumed.
    fn cloned_request_builder(&self) -> Result<RequestBuilder> {
        self.request_builder.try_clone().ok_or_else(|| {
            OciDistributionError::GenericError(Some("could not clone request builder".to_string()))
        })
    }

    fn apply_accept(&self, accept: &[&str]) -> Result<RequestBuilderWrapper<'_>> {
        let request_builder = self
            .cloned_request_builder()?
            .header("Accept", Vec::from(accept).join(", "));

        Ok(RequestBuilderWrapper {
            client: self.client,
            request_builder,
        })
    }

    /// Returns whether the request being built is addressed to the registry the
    /// credentials of `image` belong to.
    ///
    /// The upload `Location` returned by a registry may be absolute and point at
    /// another host (e.g. a signed URL of a cloud storage provider), which the
    /// distribution specification permits. Sending the `Authorization` header
    /// there is what it does not permit: clients "MUST NOT forward Authorization
    /// headers across host boundaries unless explicitly configured to do so".
    /// See also CVE-2020-15157.
    ///
    /// A same-host location that drops back from https to http is treated the
    /// same way, since the credentials would otherwise go out in the clear.
    fn targets_credential_registry(&self, image: &Reference) -> Result<bool> {
        let request = self.cloned_request_builder()?.build()?;
        let target = request.url();

        let registry = image.resolve_registry();
        let registry_url = Url::parse(&format!(
            "{scheme}://{registry}",
            scheme = self.client.config.protocol.scheme_for(registry)
        ))
        .map_err(|e| OciDistributionError::UrlParseError(e.to_string()))?;

        if target.host_str() != registry_url.host_str() {
            return Ok(false);
        }
        if target.port_or_known_default() != registry_url.port_or_known_default() {
            return Ok(false);
        }
        // The registry is reached over https, the credentials must not go out
        // in the clear.
        if registry_url.scheme() == "https" && target.scheme() != "https" {
            return Ok(false);
        }

        Ok(true)
    }

    /// Updates request as necessary for authentication.
    ///
    /// If the struct has Some(bearer), this will insert the bearer token in an
    /// Authorization header. It will also set the Accept header, which must
    /// be set on all OCI Registry requests. If the struct has HTTP Basic Auth
    /// credentials, these will be configured.
    ///
    /// Requests addressed to a host other than the registry the credentials
    /// belong to are left unauthenticated, see
    /// [`Self::targets_credential_registry`].
    async fn apply_auth(
        &self,
        image: &Reference,
        op: RegistryOperation,
    ) -> Result<RequestBuilderWrapper<'_>> {
        // NOTE: we must not authenticate requests addressed outside of the
        // registry, as those can be abused to leak credentials or tokens.
        // Please refer to CVE-2020-15157 for more information.
        if !self.targets_credential_registry(image)? {
            debug!(
                registry = image.resolve_registry(),
                "Not authenticating a request addressed outside of the registry"
            );
            return Ok(RequestBuilderWrapper {
                client: self.client,
                request_builder: self.cloned_request_builder()?,
            });
        }

        let mut headers = HeaderMap::new();
        if let Some(token) = self.client.get_auth_token(image, op).await {
            match token {
                RegistryTokenType::Bearer(token) => {
                    debug!("Using bearer token authentication.");
                    headers.insert("Authorization", token.bearer_token().parse().unwrap());
                }
                RegistryTokenType::Basic(username, password) => {
                    debug!("Using HTTP basic authentication.");
                    return Ok(RequestBuilderWrapper {
                        client: self.client,
                        request_builder: self
                            .cloned_request_builder()?
                            .headers(headers)
                            .basic_auth(username.to_string(), Some(password.to_string())),
                    });
                }
            }
        }
        Ok(RequestBuilderWrapper {
            client: self.client,
            request_builder: self.cloned_request_builder()?.headers(headers),
        })
    }
}

/// The encoding of the certificate
#[derive(Debug, Clone)]
pub enum CertificateEncoding {
    #[allow(missing_docs)]
    Der,
    #[allow(missing_docs)]
    Pem,
}

/// A x509 certificate
#[derive(Debug, Clone)]
pub struct Certificate {
    /// Which encoding is used by the certificate
    pub encoding: CertificateEncoding,

    /// Actual certificate
    pub data: Vec<u8>,
}

impl TryFrom<&Certificate> for reqwest::Certificate {
    type Error = OciDistributionError;

    fn try_from(cert: &Certificate) -> Result<Self> {
        match cert.encoding {
            CertificateEncoding::Der => Ok(reqwest::Certificate::from_der(cert.data.as_slice())?),
            CertificateEncoding::Pem => Ok(reqwest::Certificate::from_pem(cert.data.as_slice())?),
        }
    }
}

fn convert_certificates(certs: &[Certificate]) -> Result<Vec<reqwest::Certificate>> {
    certs.iter().map(reqwest::Certificate::try_from).collect()
}

/// A client configuration
pub struct ClientConfig {
    /// Which protocol the client should use
    pub protocol: ClientProtocol,

    /// Accept invalid hostname. Defaults to false
    #[cfg(feature = "native-tls")]
    pub accept_invalid_hostnames: bool,

    /// Accept invalid certificates. Defaults to false
    pub accept_invalid_certificates: bool,

    /// Use monolithic push for pushing blobs. Defaults to false
    pub use_monolithic_push: bool,

    /// Use only the provided certificate roots.
    ///
    /// This option disables any native or built-in roots, and **only** uses
    /// the roots provided to this method.
    pub tls_certs_only: Vec<Certificate>,

    /// A list of extra root certificate to trust. This can be used to connect
    /// to servers using self-signed certificates
    pub extra_root_certificates: Vec<Certificate>,

    /// A function that defines the client's behaviour if an Image Index Manifest
    /// (i.e Manifest List) is encountered when pulling an image.
    /// Defaults to [current_platform_resolver],
    /// which attempts to choose an image matching the running OS and Arch.
    ///
    /// If set to None, an error is raised if an Image Index manifest is received
    /// during an image pull.
    pub platform_resolver: Option<Box<PlatformResolverFn>>,

    /// Maximum number of concurrent uploads to perform during a `push`
    /// operation.
    ///
    /// This defaults to [`DEFAULT_MAX_CONCURRENT_UPLOAD`].
    pub max_concurrent_upload: usize,

    /// Maximum number of concurrent downloads to perform during a `pull`
    /// operation.
    ///
    /// This defaults to [`DEFAULT_MAX_CONCURRENT_DOWNLOAD`].
    pub max_concurrent_download: usize,

    /// Default token expiration in seconds, to use when the token claim
    /// doesn't provide a value.
    ///
    /// This defaults to [`DEFAULT_TOKEN_EXPIRATION_SECS`].
    pub default_token_expiration_secs: usize,

    /// Enables a read timeout for the client.
    ///
    /// See [`reqwest::ClientBuilder::read_timeout`] for more information.
    pub read_timeout: Option<Duration>,

    /// Set a timeout for the connect phase for the client.
    ///
    /// See [`reqwest::ClientBuilder::connect_timeout`] for more information.
    pub connect_timeout: Option<Duration>,

    /// Sets a total request timeout, including response body transfer.
    pub total_timeout: Option<Duration>,

    /// Maximum number of redirects accepted for one request.
    pub max_redirects: usize,

    /// Maximum number of bytes buffered for manifests, auth and error responses.
    pub max_response_bytes: usize,

    /// Additional hosts that may serve bearer authentication realms.
    /// The registry host is always allowed; an empty list therefore remains safe.
    pub allowed_auth_realm_hosts: Vec<String>,

    /// Set the `User-Agent` used by the client.
    ///
    /// This defaults to `oci-client/<version>` where `<version>` is the crate version.
    pub user_agent: &'static str,

    /// Set the `HTTPS PROXY` used by the client.
    ///
    /// This defaults to `None`.
    pub https_proxy: Option<String>,

    /// Set the `HTTP PROXY` used by the client.
    ///
    /// This defaults to `None`.
    pub http_proxy: Option<String>,

    /// Set the `NO PROXY` used by the client.
    ///
    /// This defaults to `None`.
    pub no_proxy: Option<String>,
}

impl Default for ClientConfig {
    fn default() -> Self {
        Self {
            protocol: ClientProtocol::default(),
            #[cfg(feature = "native-tls")]
            accept_invalid_hostnames: false,
            accept_invalid_certificates: false,
            use_monolithic_push: false,
            tls_certs_only: Vec::new(),
            extra_root_certificates: Vec::new(),
            platform_resolver: Some(Box::new(current_platform_resolver)),
            max_concurrent_upload: DEFAULT_MAX_CONCURRENT_UPLOAD,
            max_concurrent_download: DEFAULT_MAX_CONCURRENT_DOWNLOAD,
            default_token_expiration_secs: DEFAULT_TOKEN_EXPIRATION_SECS,
            read_timeout: None,
            connect_timeout: None,
            total_timeout: Some(Duration::from_secs(60)),
            max_redirects: 3,
            max_response_bytes: 4 * 1024 * 1024,
            allowed_auth_realm_hosts: Vec::new(),
            user_agent: DEFAULT_USER_AGENT,
            https_proxy: None,
            http_proxy: None,
            no_proxy: None,
        }
    }
}

// Be explicit about the traits supported by this type. This is needed to use
// the Client behind a dynamic reference.
// Something similar to what is described here: https://users.rust-lang.org/t/how-to-send-function-closure-to-another-thread/43549
type PlatformResolverFn = dyn Fn(&[ImageIndexEntry]) -> Option<String> + Send + Sync;

/// A platform resolver that chooses the first linux/amd64 variant, if present
pub fn linux_amd64_resolver(manifests: &[ImageIndexEntry]) -> Option<String> {
    manifests
        .iter()
        .find(|entry| {
            entry.platform.as_ref().is_some_and(|platform| {
                platform.os == Os::Linux && platform.architecture == Arch::Amd64
            })
        })
        .map(|entry| entry.digest.clone())
}

/// A platform resolver that chooses the first windows/amd64 variant, if present
pub fn windows_amd64_resolver(manifests: &[ImageIndexEntry]) -> Option<String> {
    manifests
        .iter()
        .find(|entry| {
            entry.platform.as_ref().is_some_and(|platform| {
                platform.os == Os::Windows && platform.architecture == Arch::Amd64
            })
        })
        .map(|entry| entry.digest.clone())
}

/// A platform resolver that chooses the first variant matching the running OS/Arch, if present.
/// Doesn't currently handle platform.variants.
pub fn current_platform_resolver(manifests: &[ImageIndexEntry]) -> Option<String> {
    manifests
        .iter()
        .find(|entry| {
            entry.platform.as_ref().is_some_and(|platform| {
                platform.os == Os::default() && platform.architecture == Arch::default()
            })
        })
        .map(|entry| entry.digest.clone())
}

/// The protocol that the client should use to connect
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum ClientProtocol {
    #[allow(missing_docs)]
    Http,
    #[allow(missing_docs)]
    #[default]
    Https,
    #[allow(missing_docs)]
    HttpsExcept(Vec<String>),
}

impl ClientProtocol {
    fn scheme_for(&self, registry: &str) -> &str {
        match self {
            ClientProtocol::Https => "https",
            ClientProtocol::Http => "http",
            ClientProtocol::HttpsExcept(exceptions) => {
                if exceptions.contains(&registry.to_owned()) {
                    "http"
                } else {
                    "https"
                }
            }
        }
    }
}

#[derive(Clone, Debug)]
struct BearerChallenge {
    pub realm: Box<str>,
    pub service: Option<String>,
}

impl TryFrom<&HeaderValue> for BearerChallenge {
    type Error = String;

    fn try_from(value: &HeaderValue) -> std::result::Result<Self, Self::Error> {
        let parser = ChallengeParser::new(
            value
                .to_str()
                .map_err(|e| format!("cannot convert header value to string: {e:?}"))?,
        );
        parser
            .filter_map(|parser_res| {
                if let Ok(chalenge_ref) = parser_res {
                    let bearer_challenge = BearerChallenge::try_from(&chalenge_ref);
                    bearer_challenge.ok()
                } else {
                    None
                }
            })
            .next()
            .ok_or_else(|| "Cannot find Bearer challenge".to_string())
    }
}
