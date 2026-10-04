//! Request-scoped credentials arrive through a private descriptor, never API JSON.
use anyhow::{Result, ensure};
use oci_client::{Reference, secrets::RegistryAuth};
use serde::Deserialize;
use std::{fs::File, io::Read};
use zeroize::{Zeroize, Zeroizing};

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Credentials {
    #[serde(default)]
    pub(crate) registry: String,
    #[serde(default)]
    username: String,
    #[serde(default)]
    password: String,
    #[serde(default)]
    token: String,
    #[serde(default)]
    pub(crate) auth_authorities: Vec<String>,
    #[serde(default)]
    pub(crate) ca_pem: String,
}
impl Drop for Credentials {
    fn drop(&mut self) {
        self.username.zeroize();
        self.password.zeroize();
        self.token.zeroize();
    }
}
impl Credentials {
    pub fn read(file: Option<File>) -> Result<Self> {
        let Some(file) = file else {
            return Ok(Self::default());
        };
        let stat = rustix::fs::fstat(&file)?;
        ensure!(
            rustix::fs::FileType::from_raw_mode(stat.st_mode) == rustix::fs::FileType::RegularFile
                && stat.st_uid == rustix::process::geteuid().as_raw()
                && stat.st_mode & 0o077 == 0
                && stat.st_nlink == 1
                && (0..=65536).contains(&stat.st_size),
            "credential descriptor must be a private bounded regular file"
        );
        let mut bytes = Zeroizing::new(Vec::new());
        file.take(65537).read_to_end(&mut bytes)?;
        ensure!(bytes.len() <= 65536, "credential provider exceeds limit");
        // Never include parse errors: malformed JSON can quote secret bytes.
        serde_json::from_slice(&bytes).map_err(|_| anyhow::anyhow!("invalid credential provider"))
    }
    pub(crate) fn auth(&self, reference: &Reference) -> Result<RegistryAuth> {
        let supplied =
            !self.username.is_empty() || !self.password.is_empty() || !self.token.is_empty();
        ensure!(
            !supplied || self.registry == reference.resolve_registry(),
            "credential authority mismatch"
        );
        ensure!(
            self.token.is_empty() || (self.username.is_empty() && self.password.is_empty()),
            "ambiguous credential provider"
        );
        ensure!(
            self.auth_authorities.len() <= 16,
            "too many credential authorities"
        );
        if !self.token.is_empty() {
            Ok(RegistryAuth::Bearer(self.token.clone()))
        } else if supplied {
            ensure!(!self.username.is_empty(), "missing credential username");
            Ok(RegistryAuth::Basic(
                self.username.clone(),
                self.password.clone(),
            ))
        } else {
            Ok(RegistryAuth::Anonymous)
        }
    }
}

pub(crate) fn reference(value: &str, pinned: bool) -> Result<Reference> {
    ensure!(
        value.len() <= 512
            && !value.is_empty()
            && value
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"/:._-@".contains(&b)),
        "invalid registry reference"
    );
    let reference: Reference = value
        .parse()
        .map_err(|_| anyhow::anyhow!("invalid registry reference"))?;
    if pinned {
        let _: artifactd_protocol::ArtifactDigest = reference
            .digest()
            .ok_or_else(|| anyhow::anyhow!("pull requires a SHA-256 digest reference"))?
            .parse()?;
    } else if let Some(digest) = reference.digest() {
        let _: artifactd_protocol::ArtifactDigest = digest.parse()?;
    }
    Ok(reference)
}
