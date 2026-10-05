use crate::state::PeerIdentity;
use anyhow::{Result, ensure};
use artifactd_protocol::{Action, Request};
use rustix::{
    fs::{self, Mode, OFlags},
    process::{getegid, geteuid},
};
use serde::Deserialize;
use std::{collections::BTreeMap, fs::File, io::Read, path::Path};

const MAX_BYTES: u64 = 64 * 1024;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Role {
    Producer,
    Consumer,
    Admin,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FilePolicy {
    socket_gid: u32,
    peers: Vec<PeerEntry>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PeerEntry {
    uid: u32,
    gid: u32,
    role: Role,
}

pub(crate) struct Policy {
    socket_gid: Option<u32>,
    peers: BTreeMap<(u32, u32), Role>,
}

impl Policy {
    pub(crate) fn load(path: Option<&Path>) -> Result<Self> {
        let service = PeerIdentity {
            uid: geteuid().as_raw(),
            gid: getegid().as_raw(),
        };
        let Some(path) = path else {
            let mut peers = BTreeMap::new();
            peers.insert((service.uid, service.gid), Role::Admin);
            return Ok(Self {
                socket_gid: None,
                peers,
            });
        };
        ensure!(path.is_absolute(), "policy path must be absolute");
        let fd = fs::open(
            path,
            OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )?;
        let stat = fs::fstat(&fd)?;
        ensure!(
            fs::FileType::from_raw_mode(stat.st_mode) == fs::FileType::RegularFile
                && stat.st_uid == service.uid
                && stat.st_nlink == 1
                && stat.st_mode & 0o022 == 0
                && (0..=MAX_BYTES as i64).contains(&stat.st_size),
            "policy must be a private bounded regular file"
        );
        let mut bytes = Vec::with_capacity(stat.st_size as usize);
        File::from(fd).take(MAX_BYTES + 1).read_to_end(&mut bytes)?;
        ensure!(bytes.len() <= MAX_BYTES as usize, "policy exceeds limit");
        let parsed: FilePolicy =
            serde_json::from_slice(&bytes).map_err(|_| anyhow::anyhow!("invalid policy"))?;
        ensure!(
            !parsed.peers.is_empty() && parsed.peers.len() <= 64,
            "invalid policy peer count"
        );
        let mut peers = BTreeMap::new();
        for entry in parsed.peers {
            ensure!(
                peers.insert((entry.uid, entry.gid), entry.role).is_none(),
                "duplicate policy peer"
            );
        }
        ensure!(
            peers.get(&(service.uid, service.gid)) == Some(&Role::Admin),
            "policy must grant the service peer admin"
        );
        Ok(Self {
            socket_gid: Some(parsed.socket_gid),
            peers,
        })
    }

    pub(crate) fn socket_gid(&self) -> Option<u32> {
        self.socket_gid
    }

    pub(crate) fn authorize_peer(&self, caller: &PeerIdentity) -> Result<Role> {
        self.peers
            .get(&(caller.uid, caller.gid))
            .copied()
            .ok_or_else(|| anyhow::anyhow!("peer is not authorized"))
    }

    pub(crate) fn authorize_request(&self, request: &Request, caller: &PeerIdentity) -> Result<()> {
        if let Action::LeaseCreate { grantee, .. } = &request.action {
            let requested = grantee.as_ref().map(|peer| PeerIdentity {
                uid: peer.uid,
                gid: peer.gid,
            });
            self.resolve_grantee(caller, requested.as_ref())?;
        }
        let role = self.authorize_peer(caller)?;
        if role == Role::Admin {
            return Ok(());
        }
        let allowed = matches!(
            (&request.action, role),
            (
                Action::OperationAllocate
                    | Action::Inspect { .. }
                    | Action::Verify { .. }
                    | Action::Resolve { .. }
                    | Action::EnsureLocal { .. }
                    | Action::Status
                    | Action::Capabilities,
                _,
            ) | (
                Action::ImportBlob { .. }
                    | Action::ImportOci { .. }
                    | Action::ImportOciArchive { .. }
                    | Action::Pin { .. }
                    | Action::Unpin { .. }
                    | Action::Prepare { .. }
                    | Action::Pull { .. }
                    | Action::Push { .. },
                Role::Producer,
            ) | (
                Action::LeaseCreate { .. }
                    | Action::LeaseRelease { .. }
                    | Action::OpenBlob { .. }
                    | Action::OpenPrepared { .. },
                Role::Producer | Role::Consumer,
            )
        );
        ensure!(allowed, "peer role is not authorized for this operation");
        Ok(())
    }

    pub(crate) fn resolve_grantee(
        &self,
        caller: &PeerIdentity,
        requested: Option<&PeerIdentity>,
    ) -> Result<PeerIdentity> {
        let role = self.authorize_peer(caller)?;
        let target = requested.cloned().unwrap_or_else(|| caller.clone());
        ensure!(
            target == *caller || role == Role::Producer || role == Role::Admin,
            "consumer may only create a lease for itself"
        );
        let target_role = self.authorize_peer(&target)?;
        ensure!(
            target_role == Role::Consumer || target_role == Role::Admin || target == *caller,
            "lease grantee is not an authorized consumer"
        );
        Ok(target)
    }
}
