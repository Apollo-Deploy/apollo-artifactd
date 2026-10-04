//! Versioned generic artifact contract. Files cross the boundary as SCM_RIGHTS.
mod digest;
pub use digest::{ArtifactDigest, BlobDigest, ConfigDigest, ManifestDigest};
use serde::{Deserialize, Serialize};

pub const VERSION: u32 = 1;
pub const MAX_PACKET: usize = 64 * 1024;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Platform {
    pub os: String,
    pub architecture: String,
    pub variant: Option<String>,
}

impl Platform {
    pub fn validate(&self) -> bool {
        self.os == "linux"
            && matches!(self.architecture.as_str(), "amd64" | "arm64")
            && self
                .variant
                .as_ref()
                .is_none_or(|v| v.len() <= 32 && v.bytes().all(|c| c.is_ascii_alphanumeric()))
    }
}

macro_rules! id {
    ($name:ident) => {
        #[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
        #[serde(try_from = "String", into = "String")]
        pub struct $name(String);
        impl TryFrom<String> for $name {
            type Error = &'static str;
            fn try_from(value: String) -> Result<Self, Self::Error> {
                if value.is_empty()
                    || value.len() > 128
                    || !value
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
                {
                    return Err("invalid opaque identity");
                }
                Ok(Self(value))
            }
        }
        impl From<$name> for String {
            fn from(v: $name) -> Self {
                v.0
            }
        }
        impl $name {
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }
    };
}
id!(OperationId);
id!(PinId);
id!(LeaseId);
id!(PreparedArtifactId);

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub version: u32,
    pub operation_id: OperationId,
    pub action: Action,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(
    tag = "operation",
    rename_all = "SCREAMING_SNAKE_CASE",
    deny_unknown_fields
)]
pub enum Action {
    ImportBlob {
        digest: BlobDigest,
        size: u64,
    },
    ImportOci {
        digest: ArtifactDigest,
        platform: Platform,
    },
    ImportOciArchive {
        platform: Platform,
    },
    Inspect {
        digest: ArtifactDigest,
    },
    Verify {
        digest: ArtifactDigest,
    },
    Resolve {
        digest: ArtifactDigest,
        platform: Platform,
    },
    Pin {
        id: PinId,
        digest: ArtifactDigest,
    },
    Unpin {
        id: PinId,
    },
    LeaseCreate {
        id: LeaseId,
        digest: ArtifactDigest,
    },
    LeaseRelease {
        id: LeaseId,
    },
    Prepare {
        digest: ManifestDigest,
        platform: Platform,
    },
    OpenBlob {
        digest: BlobDigest,
        lease: LeaseId,
    },
    OpenPrepared {
        id: PreparedArtifactId,
        lease: LeaseId,
    },
    Pull {
        reference: String,
        platform: Platform,
    },
    Push {
        digest: ArtifactDigest,
        reference: String,
    },
    EnsureLocal {
        digest: ArtifactDigest,
    },
    Gc {
        max_entries: u32,
    },
    Reconcile {
        max_operations: u32,
    },
    Status,
    Doctor,
    Capabilities,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Response {
    pub version: u32,
    pub operation_id: OperationId,
    pub result: Result<serde_json::Value, String>,
}

#[cfg(target_os = "linux")]
pub mod wire;

#[cfg(target_os = "linux")]
pub mod client;
