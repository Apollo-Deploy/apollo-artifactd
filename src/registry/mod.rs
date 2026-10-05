//! OCI protocol and TLS remain in the maintained oci-client library.
mod credentials;
mod pull;
mod push;
use anyhow::Result;
pub use credentials::Credentials;
#[cfg(target_os = "linux")]
pub(crate) use credentials::reference as validate_reference;
use oci_client::{
    Client,
    client::{Certificate, CertificateEncoding, ClientConfig, ClientProtocol},
};
use std::time::Duration;

/// Fixed-size runtime shared across operations; CAS runs on the calling worker.
pub struct Registry {
    runtime: tokio::runtime::Runtime,
}
impl Registry {
    pub fn new() -> Result<Self> {
        Ok(Self {
            runtime: tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .max_blocking_threads(2)
                .enable_all()
                .build()?,
        })
    }
    fn client(&self, credentials: &Credentials, metadata_limit: u64) -> Result<Client> {
        let mut config = ClientConfig {
            protocol: ClientProtocol::Https,
            connect_timeout: Some(Duration::from_secs(10)),
            read_timeout: Some(Duration::from_secs(30)),
            total_timeout: Some(Duration::from_secs(300)),
            max_redirects: 3,
            max_response_bytes: usize::try_from(metadata_limit)?,
            max_concurrent_download: 2,
            max_concurrent_upload: 2,
            allowed_auth_realm_hosts: credentials.auth_authorities.clone(),
            use_monolithic_push: true,
            ..Default::default()
        };
        if !credentials.ca_pem.is_empty() {
            config.extra_root_certificates.push(Certificate {
                encoding: CertificateEncoding::Pem,
                data: credentials.ca_pem.as_bytes().to_vec(),
            });
        }
        // Construction may create runtime resources. It is always inside this runtime.
        let _guard = self.runtime.enter();
        Client::try_from(config)
            .map_err(|_| anyhow::anyhow!("registry client configuration failed"))
    }
}

fn remote<T>(result: oci_client::errors::Result<T>) -> Result<T> {
    // Registry error bodies and URLs can contain credentials. Keep them out of
    // operation journals, API responses and logs.
    result.map_err(|_| anyhow::anyhow!("registry request failed"))
}
