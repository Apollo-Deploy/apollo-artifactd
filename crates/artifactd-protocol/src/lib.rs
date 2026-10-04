//! Versioned generic artifact contract. Files cross the boundary as SCM_RIGHTS.
mod digest;
pub use digest::{ArtifactDigest, BlobDigest, ConfigDigest, ManifestDigest, PreparedDigest};
use serde::{Deserialize, Serialize};

pub const VERSION: u32 = 2;
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

impl OperationId {
    pub fn token(epoch: &str, sequence: u64) -> Result<Self, &'static str> {
        if sequence == 0
            || epoch.len() != 32
            || !epoch
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
        {
            return Err("invalid operation token");
        }
        Self::try_from(format!("{epoch}_{sequence:020}")).map_err(|_| "invalid operation token")
    }

    pub fn token_parts(&self) -> Result<(&str, u64), &'static str> {
        let (epoch, sequence) = self.0.split_once('_').ok_or("invalid operation token")?;
        if epoch.len() != 32
            || !epoch
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
            || sequence.len() != 20
            || !sequence.bytes().all(|byte| byte.is_ascii_digit())
        {
            return Err("invalid operation token");
        }
        let sequence = sequence.parse().map_err(|_| "invalid operation token")?;
        if sequence == 0 {
            return Err("invalid operation token");
        }
        Ok((epoch, sequence))
    }
}

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
    OperationAllocate,
    ImportBlob {
        #[serde(default)]
        digest: Option<BlobDigest>,
        size: u64,
    },
    ImportOci {
        digest: ArtifactDigest,
        platform: Platform,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pin: Option<PinId>,
    },
    ImportOciArchive {
        platform: Platform,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pin: Option<PinId>,
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
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pin: Option<PinId>,
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

impl Action {
    pub fn is_mutation(&self) -> bool {
        !matches!(
            self,
            Self::OperationAllocate
                | Self::Inspect { .. }
                | Self::Verify { .. }
                | Self::Resolve { .. }
                | Self::OpenBlob { .. }
                | Self::OpenPrepared { .. }
                | Self::Status
                | Self::Capabilities
        )
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Response {
    pub version: u32,
    pub operation_id: OperationId,
    pub result: Result<serde_json::Value, String>,
}

#[cfg(test)]
mod tests {
    use super::{Action, OperationId};

    #[test]
    fn operation_tokens_are_canonical_and_round_trip() {
        let token = OperationId::token("0123456789abcdef0123456789abcdef", 1).unwrap();
        assert_eq!(
            token.as_str(),
            "0123456789abcdef0123456789abcdef_00000000000000000001"
        );
        assert_eq!(
            token.token_parts().unwrap(),
            ("0123456789abcdef0123456789abcdef", 1)
        );
        assert!(OperationId::token("0123456789ABCDEF0123456789abcdef", 1).is_err());
        assert!(OperationId::token("0123456789abcdef0123456789abcdef", 0).is_err());
    }

    #[test]
    fn mutation_classification_keeps_observations_and_allocation_read_only() {
        assert!(!Action::OperationAllocate.is_mutation());
        assert!(!Action::Status.is_mutation());
        assert!(
            !Action::OpenPrepared {
                id: "prepared".to_string().try_into().unwrap(),
                lease: "lease".to_string().try_into().unwrap(),
            }
            .is_mutation()
        );
        assert!(Action::Gc { max_entries: 1 }.is_mutation());
    }
}

#[cfg(target_os = "linux")]
pub mod wire;

#[cfg(target_os = "linux")]
pub mod client;
