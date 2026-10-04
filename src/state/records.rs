//! Durable records shared by the redb state adapter and its callers.
use artifactd_protocol::{ArtifactDigest, Platform};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Blob {
    pub size: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Reference {
    pub digest: ArtifactDigest,
    /// Legacy references remain unowned and require explicit migration.
    #[serde(default)]
    pub owner: Option<PeerIdentity>,
}

impl Reference {
    pub fn authorize(&self, caller: &PeerIdentity) -> anyhow::Result<()> {
        self.require_owner()?;
        anyhow::ensure!(
            self.owner.as_ref() == Some(caller),
            "reference owner mismatch"
        );
        Ok(())
    }

    pub fn require_owner(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.owner.is_some(),
            "legacy reference ownership requires explicit migration"
        );
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Garbage {
    pub size: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Imported {
    pub temp: String,
    pub digest: Option<ArtifactDigest>,
    pub size: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Root {
    pub kind: String,
    pub platform: Option<Platform>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Prepared {
    pub manifest: ArtifactDigest,
    pub platform: Platform,
    pub phase: String,
    pub tree_digest: Option<String>,
    pub staging: String,
    pub size: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PeerIdentity {
    pub uid: u32,
    pub gid: u32,
}

impl PeerIdentity {
    pub fn current() -> Self {
        Self {
            uid: rustix::process::geteuid().as_raw(),
            gid: rustix::process::getegid().as_raw(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Operation {
    pub request: String,
    pub phase: String,
    pub result: Option<String>,
    pub sequence: u64,
    /// Missing on legacy records; never adopt these on first use.
    #[serde(default)]
    pub owner: Option<PeerIdentity>,
}
