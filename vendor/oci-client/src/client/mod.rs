//! OCI distribution client for fetching oci images from an OCI compliant remote store
use std::collections::{BTreeMap, HashMap};
use std::convert::TryFrom;
use std::hash::Hash;
use std::pin::pin;
use std::sync::Arc;
use std::time::Duration;

use bytes::BytesMut;
use futures_util::stream::{self, BoxStream, StreamExt, TryStreamExt};
use futures_util::{future, Stream};
use http::header::RANGE;
use http::{HeaderValue, StatusCode};
use http_auth::{parser::ChallengeParser, ChallengeRef};
use oci_spec::image::{Arch, Os};
use olpc_cjson::CanonicalFormatter;
use reqwest::header::HeaderMap;
use reqwest::{NoProxy, Proxy, RequestBuilder, Response, Url};
use serde::{Deserialize, Deserializer, Serialize};
use sha2::Digest as _;
use tokio::io::{AsyncWrite, AsyncWriteExt};
use tokio::sync::RwLock;
use tracing::{debug, trace, warn};

pub use crate::blob::*;
use crate::config::ConfigFile;
use crate::digest::{digest_header_value, validate_digest, Digest, Digester};
use crate::errors::*;
use crate::manifest::{
    ImageIndexEntry, OciDescriptor, OciImageIndex, OciImageManifest, OciManifest, Versioned,
    IMAGE_CONFIG_MEDIA_TYPE, IMAGE_LAYER_GZIP_MEDIA_TYPE, IMAGE_LAYER_MEDIA_TYPE,
    IMAGE_MANIFEST_LIST_MEDIA_TYPE, IMAGE_MANIFEST_MEDIA_TYPE, OCI_IMAGE_INDEX_MEDIA_TYPE,
    OCI_IMAGE_MEDIA_TYPE,
};
use crate::secrets::RegistryAuth;
use crate::secrets::*;
use crate::sha256_digest;
use crate::token_cache::{RegistryOperation, RegistryToken, RegistryTokenType, TokenCache};
use crate::Reference;

const MIME_TYPES_DISTRIBUTION_MANIFEST: &[&str] = &[
    IMAGE_MANIFEST_MEDIA_TYPE,
    IMAGE_MANIFEST_LIST_MEDIA_TYPE,
    OCI_IMAGE_MEDIA_TYPE,
    OCI_IMAGE_INDEX_MEDIA_TYPE,
];

const PUSH_CHUNK_MAX_SIZE: usize = 4096 * 1024;

/// Default value for `ClientConfig::max_concurrent_upload`
pub const DEFAULT_MAX_CONCURRENT_UPLOAD: usize = 16;

/// Default value for `ClientConfig::max_concurrent_download`
pub const DEFAULT_MAX_CONCURRENT_DOWNLOAD: usize = 16;

/// Default value for `ClientConfig:default_token_expiration_secs`
pub const DEFAULT_TOKEN_EXPIRATION_SECS: usize = 60;

/// Reads non-blob registry responses with a hard cumulative byte limit.
async fn read_bounded_response(response: Response, limit: usize) -> Result<bytes::Bytes> {
    if response
        .content_length()
        .is_some_and(|length| length > limit as u64)
    {
        return Err(OciDistributionError::SpecViolationError(
            "registry response exceeds configured byte limit".to_string(),
        ));
    }
    let mut stream = response.bytes_stream();
    let mut body = BytesMut::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        if body.len().saturating_add(chunk.len()) > limit {
            return Err(OciDistributionError::SpecViolationError(
                "registry response exceeds configured byte limit".to_string(),
            ));
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body.freeze())
}

static DEFAULT_USER_AGENT: &str = concat!(env!("CARGO_PKG_NAME"), "/", env!("CARGO_PKG_VERSION"));

/// The data for an image or module.
#[derive(Clone)]
pub struct ImageData {
    /// The layers of the image or module.
    pub layers: Vec<ImageLayer>,
    /// The digest of the image or module.
    pub digest: Option<String>,
    /// The Configuration object of the image or module.
    pub config: Config,
    /// The manifest of the image or module.
    pub manifest: Option<OciImageManifest>,
}

/// The data returned by an OCI registry after a successful push
/// operation is completed
pub struct PushResponse {
    /// Pullable url for the config
    pub config_url: String,
    /// Pullable url for the manifest
    pub manifest_url: String,
}

/// The data returned by [`Client::push_blob_stream_chunked`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PushBlobStreamChunkedResponse {
    /// Pullable url for the uploaded blob.
    pub blob_url: String,
    /// Computed digest of the uploaded blob.
    pub blob_digest: String,
    /// Total uploaded blob size in bytes.
    pub size: u64,
}

/// The data returned by a successful tags/list Request
#[derive(Deserialize, Debug)]
pub struct TagResponse {
    /// Repository Name
    pub name: String,
    /// List of existing Tags
    #[serde(deserialize_with = "null_as_default")]
    pub tags: Vec<String>,
}

/// Helper to deserialize an empty value from a JSON `null`.
fn null_as_default<'de, D, T>(d: D) -> std::result::Result<T, D::Error>
where
    D: Deserializer<'de>,
    T: Default + Deserialize<'de>,
{
    let res = <Option<T>>::deserialize(d)?.unwrap_or_default();
    Ok(res)
}

/// The data returned by a successful catalog request.
#[derive(Deserialize, Debug)]
pub struct CatalogResponse {
    /// List of available repositories in the registry.
    pub repositories: Vec<String>,
}

/// Layer descriptor required to pull a layer
pub struct LayerDescriptor<'a> {
    /// The digest of the layer
    pub digest: &'a str,
    /// Optional list of additional URIs to pull the layer from
    pub urls: &'a Option<Vec<String>>,
}

/// A trait for converting any type into a [`LayerDescriptor`]
pub trait AsLayerDescriptor {
    /// Convert the type to a LayerDescriptor reference
    fn as_layer_descriptor(&self) -> LayerDescriptor<'_>;
}

impl<T: AsLayerDescriptor> AsLayerDescriptor for &T {
    fn as_layer_descriptor(&self) -> LayerDescriptor<'_> {
        (*self).as_layer_descriptor()
    }
}

impl AsLayerDescriptor for &str {
    fn as_layer_descriptor(&self) -> LayerDescriptor<'_> {
        LayerDescriptor {
            digest: self,
            urls: &None,
        }
    }
}

impl AsLayerDescriptor for &OciDescriptor {
    fn as_layer_descriptor(&self) -> LayerDescriptor<'_> {
        LayerDescriptor {
            digest: &self.digest,
            urls: &self.urls,
        }
    }
}

impl AsLayerDescriptor for &LayerDescriptor<'_> {
    fn as_layer_descriptor(&self) -> LayerDescriptor<'_> {
        LayerDescriptor {
            digest: self.digest,
            urls: self.urls,
        }
    }
}

/// The data and media type for an image layer
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct ImageLayer {
    /// The data of this layer
    pub data: bytes::Bytes,
    /// The media type of this layer
    pub media_type: String,
    /// This OPTIONAL property contains arbitrary metadata for this descriptor.
    /// This OPTIONAL property MUST use the [annotation rules](https://github.com/opencontainers/image-spec/blob/main/annotations.md#rules)
    pub annotations: Option<BTreeMap<String, String>>,
}

impl ImageLayer {
    /// Constructs a new ImageLayer struct with provided data and media type
    pub fn new(
        data: impl Into<bytes::Bytes>,
        media_type: String,
        annotations: Option<BTreeMap<String, String>>,
    ) -> Self {
        ImageLayer {
            data: data.into(),
            media_type,
            annotations,
        }
    }

    /// Constructs a new ImageLayer struct with provided data and
    /// media type application/vnd.oci.image.layer.v1.tar
    pub fn oci_v1(
        data: impl Into<bytes::Bytes>,
        annotations: Option<BTreeMap<String, String>>,
    ) -> Self {
        Self::new(data, IMAGE_LAYER_MEDIA_TYPE.to_string(), annotations)
    }
    /// Constructs a new ImageLayer struct with provided data and
    /// media type application/vnd.oci.image.layer.v1.tar+gzip
    pub fn oci_v1_gzip(
        data: impl Into<bytes::Bytes>,
        annotations: Option<BTreeMap<String, String>>,
    ) -> Self {
        Self::new(data, IMAGE_LAYER_GZIP_MEDIA_TYPE.to_string(), annotations)
    }

    /// Helper function to compute the sha256 digest of an image layer
    pub fn sha256_digest(&self) -> String {
        sha256_digest(&self.data)
    }
}

/// The data and media type for a configuration object
#[derive(Clone)]
pub struct Config {
    /// The data of this config object
    pub data: bytes::Bytes,
    /// The media type of this object
    pub media_type: String,
    /// This OPTIONAL property contains arbitrary metadata for this descriptor.
    /// This OPTIONAL property MUST use the [annotation rules](https://github.com/opencontainers/image-spec/blob/main/annotations.md#rules)
    pub annotations: Option<BTreeMap<String, String>>,
}

impl Config {
    /// Constructs a new Config struct with provided data and media type
    pub fn new(
        data: impl Into<bytes::Bytes>,
        media_type: String,
        annotations: Option<BTreeMap<String, String>>,
    ) -> Self {
        Config {
            data: data.into(),
            media_type,
            annotations,
        }
    }

    /// Constructs a new Config struct with provided data and
    /// media type application/vnd.oci.image.config.v1+json
    pub fn oci_v1(
        data: impl Into<bytes::Bytes>,
        annotations: Option<BTreeMap<String, String>>,
    ) -> Self {
        Self::new(data, IMAGE_CONFIG_MEDIA_TYPE.to_string(), annotations)
    }

    /// Construct a new Config struct with provided [`ConfigFile`] and
    /// media type `application/vnd.oci.image.config.v1+json`
    pub fn oci_v1_from_config_file(
        config_file: ConfigFile,
        annotations: Option<BTreeMap<String, String>>,
    ) -> Result<Self> {
        let data = serde_json::to_vec(&config_file)?;
        Ok(Self::new(
            data,
            IMAGE_CONFIG_MEDIA_TYPE.to_string(),
            annotations,
        ))
    }

    /// Helper function to compute the sha256 digest of this config object
    pub fn sha256_digest(&self) -> String {
        sha256_digest(&self.data)
    }
}

impl TryFrom<Config> for ConfigFile {
    type Error = crate::errors::OciDistributionError;

    fn try_from(config: Config) -> Result<Self> {
        let config = String::from_utf8(config.data.into())
            .map_err(|e| OciDistributionError::ConfigConversionError(e.to_string()))?;
        let config_file: ConfigFile = serde_json::from_str(&config)
            .map_err(|e| OciDistributionError::ConfigConversionError(e.to_string()))?;
        Ok(config_file)
    }
}

/// The OCI client connects to an OCI registry and fetches OCI images.
///
/// An OCI registry is a container registry that adheres to the OCI Distribution
/// specification. DockerHub is one example, as are ACR and GCR. This client
/// provides a native Rust implementation for pulling OCI images.
///
/// Some OCI registries support completely anonymous access. But most require
/// at least an Oauth2 handshake. Typically, you will want to create a new
/// client, and then run the `auth()` method, which will attempt to get
/// a read-only bearer token. From there, pulling images can be done with
/// the `pull_*` functions.
///
/// For true anonymous access, you can skip `auth()`. This is not recommended
/// unless you are sure that the remote registry does not require Oauth2.
#[derive(Clone)]
pub struct Client {
    config: Arc<ClientConfig>,
    // Registry -> RegistryAuth
    auth_store: Arc<RwLock<HashMap<String, RegistryAuth>>>,
    /// Token cache for the client
    pub tokens: TokenCache,
    client: reqwest::Client,
    push_chunk_size: usize,
}

impl Default for Client {
    fn default() -> Self {
        Self {
            config: Arc::default(),
            auth_store: Arc::default(),
            tokens: TokenCache::new(DEFAULT_TOKEN_EXPIRATION_SECS),
            client: reqwest::Client::default(),
            push_chunk_size: PUSH_CHUNK_MAX_SIZE,
        }
    }
}

/// A source that can provide a `ClientConfig`.
/// If you are using this crate in your own application, you can implement this
/// trait on your configuration type so that it can be passed to `Client::from_source`.
pub trait ClientConfigSource {
    /// Provides a `ClientConfig`.
    fn client_config(&self) -> ClientConfig;
}

impl TryFrom<ClientConfig> for Client {
    type Error = OciDistributionError;

    fn try_from(config: ClientConfig) -> std::result::Result<Self, Self::Error> {
        #[allow(unused_mut)]
        let mut client_builder = reqwest::Client::builder();
        #[cfg(not(target_arch = "wasm32"))]
        let mut client_builder =
            client_builder.danger_accept_invalid_certs(config.accept_invalid_certificates);

        client_builder = match () {
            #[cfg(all(feature = "native-tls", not(target_arch = "wasm32")))]
            () => client_builder.danger_accept_invalid_hostnames(config.accept_invalid_hostnames),
            #[cfg(any(not(feature = "native-tls"), target_arch = "wasm32"))]
            () => client_builder,
        };

        #[cfg(not(target_arch = "wasm32"))]
        {
            if !config.tls_certs_only.is_empty() {
                client_builder =
                    client_builder.tls_certs_only(convert_certificates(&config.tls_certs_only)?);
            }
            client_builder = client_builder
                .tls_certs_merge(convert_certificates(&config.extra_root_certificates)?);
        }

        if let Some(timeout) = config.read_timeout {
            client_builder = client_builder.read_timeout(timeout);
        }
        if let Some(timeout) = config.connect_timeout {
            client_builder = client_builder.connect_timeout(timeout);
        }
        if let Some(timeout) = config.total_timeout {
            client_builder = client_builder.timeout(timeout);
        }

        let max_redirects = config.max_redirects;
        client_builder =
            client_builder.redirect(reqwest::redirect::Policy::custom(move |attempt| {
                if !redirect_allowed(attempt.previous(), attempt.url(), max_redirects) {
                    return attempt.error("redirect policy refused");
                }
                attempt.follow()
            }));

        client_builder = client_builder.user_agent(config.user_agent);

        if let Some(proxy_addr) = &config.https_proxy {
            let no_proxy = config
                .no_proxy
                .as_ref()
                .and_then(|no_proxy| NoProxy::from_string(no_proxy));
            let proxy = Proxy::https(proxy_addr)?.no_proxy(no_proxy);
            client_builder = client_builder.proxy(proxy);
        }

        if let Some(proxy_addr) = &config.http_proxy {
            let no_proxy = config
                .no_proxy
                .as_ref()
                .and_then(|no_proxy| NoProxy::from_string(no_proxy));
            let proxy = Proxy::http(proxy_addr)?.no_proxy(no_proxy);
            client_builder = client_builder.proxy(proxy);
        }

        let default_token_expiration_secs = config.default_token_expiration_secs;
        Ok(Self {
            config: Arc::new(config),
            tokens: TokenCache::new(default_token_expiration_secs),
            client: client_builder.build()?,
            push_chunk_size: PUSH_CHUNK_MAX_SIZE,
            ..Default::default()
        })
    }
}

include!("support.rs");
include!("policy.rs");
include!("auth_challenge.rs");

include!("foundation.rs");
include!("transfer.rs");
include!("auth.rs");
include!("manifests.rs");
include!("blobs.rs");
include!("push.rs");
