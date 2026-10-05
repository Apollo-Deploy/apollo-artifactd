use serde::{Deserialize, Serialize};
use std::{fmt, str::FromStr};

#[derive(Debug, thiserror::Error)]
#[error("identity must be sha256 followed by 64 lowercase hexadecimal digits")]
pub struct InvalidDigest;

/// Canonical immutable SHA-256 identity. Aliases describe graph roles, never tags.
#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct ArtifactDigest(String);

pub type BlobDigest = ArtifactDigest;
pub type ManifestDigest = ArtifactDigest;
pub type ConfigDigest = ArtifactDigest;
pub type PreparedDigest = ArtifactDigest;

impl ArtifactDigest {
    pub fn hex(&self) -> &str {
        &self.0[7..]
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}
impl FromStr for ArtifactDigest {
    type Err = InvalidDigest;
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        if value.len() != 71
            || !value.starts_with("sha256:")
            || !value.as_bytes()[7..]
                .iter()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(b))
        {
            return Err(InvalidDigest);
        }
        Ok(Self(value.to_owned()))
    }
}
impl TryFrom<String> for ArtifactDigest {
    type Error = InvalidDigest;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        value.parse()
    }
}
impl From<ArtifactDigest> for String {
    fn from(value: ArtifactDigest) -> Self {
        value.0
    }
}
impl fmt::Display for ArtifactDigest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}
