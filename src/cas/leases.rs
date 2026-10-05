use super::Store;
use crate::state::{LeaseState, PeerIdentity, Reference, StateTx};
use anyhow::{Result, ensure};
#[cfg(target_os = "linux")]
use artifactd_protocol::{Action, OperationId};
use artifactd_protocol::{ArtifactDigest, LeaseId};
use std::collections::BTreeSet;

const MAX_LEASES: u64 = 100_000;
const EPOCH: &str = "local_lease_epoch";
const SEQUENCE: &str = "local_lease_sequence";

impl Store {
    pub fn lease(&mut self, digest: &ArtifactDigest) -> Result<LeaseId> {
        let peer = PeerIdentity::current();
        self.open_blob(digest)?;
        self.verify_graph_if_known(digest)?;
        self.db.transaction(|tx| {
            ensure!(tx.count("leases")? < MAX_LEASES, "lease count limit");
            let existing_epoch = tx.get::<String>("metadata", EPOCH)?;
            let existing_sequence = tx.get::<u64>("metadata", SEQUENCE)?;
            let epoch = match existing_epoch {
                Some(value) => {
                    validate_epoch(&value)?;
                    ensure!(
                        existing_sequence.is_some_and(|sequence| sequence > 0),
                        "partial local lease metadata"
                    );
                    value
                }
                None => {
                    ensure!(existing_sequence.is_none(), "partial local lease metadata");
                    let value = uuid::Uuid::new_v4().simple().to_string();
                    tx.put("metadata", EPOCH, &value)?;
                    value
                }
            };
            let sequence = existing_sequence
                .unwrap_or(0)
                .checked_add(1)
                .ok_or_else(|| anyhow::anyhow!("local lease sequence exhausted"))?;
            let id = LeaseId::try_from(format!("local_lease_{epoch}_{sequence:020}"))
                .map_err(|_| anyhow::anyhow!("generated lease identity invalid"))?;
            ensure!(
                tx.get::<Reference>("leases", id.as_str())?.is_none(),
                "generated lease identity collision"
            );
            tx.put("metadata", SEQUENCE, &sequence)?;
            tx.put(
                "leases",
                id.as_str(),
                &Reference {
                    digest: digest.clone(),
                    owner: Some(peer.clone()),
                    grantee: Some(peer),
                    lease_state: Some(LeaseState::Available),
                },
            )?;
            Ok(id)
        })
    }

    #[cfg(target_os = "linux")]
    pub(crate) fn lease_for(
        &mut self,
        operation: &OperationId,
        digest: &ArtifactDigest,
        owner: &PeerIdentity,
        grantee: &PeerIdentity,
    ) -> Result<LeaseId> {
        let id = LeaseId::from_operation(operation).map_err(anyhow::Error::msg)?;
        self.open_blob(digest)?;
        self.verify_graph_if_known(digest)?;
        self.db.transaction(|tx| {
            let record = tx
                .get::<crate::state::Operation>("operations", operation.as_str())?
                .ok_or_else(|| anyhow::anyhow!("missing lease operation"))?;
            ensure!(record.phase == "intent", "lease operation is not active");
            ensure!(
                record.owner.as_ref() == Some(owner),
                "lease operation owner mismatch"
            );
            match serde_json::from_str::<Action>(&record.request)
                .map_err(|_| anyhow::anyhow!("lease operation request is invalid"))?
            {
                Action::LeaseCreate {
                    digest: requested,
                    grantee: requested_grantee,
                } => {
                    ensure!(requested == *digest, "lease operation digest mismatch");
                    let requested = requested_grantee.map(|peer| PeerIdentity {
                        uid: peer.uid,
                        gid: peer.gid,
                    });
                    ensure!(
                        requested.as_ref().unwrap_or(owner) == grantee,
                        "lease operation grantee mismatch"
                    );
                }
                _ => anyhow::bail!("operation is not a lease creation"),
            }
            if let Some(reference) = tx.get::<Reference>("leases", id.as_str())? {
                reference.validate_lease()?;
                ensure!(
                    reference.owner.as_ref() == Some(owner),
                    "lease owner mismatch"
                );
                ensure!(
                    reference.grantee.as_ref() == Some(grantee),
                    "lease grantee mismatch"
                );
                ensure!(reference.digest == *digest, "lease identity conflict");
            } else {
                ensure!(tx.count("leases")? < MAX_LEASES, "lease count limit");
                tx.put(
                    "leases",
                    id.as_str(),
                    &Reference {
                        digest: digest.clone(),
                        owner: Some(owner.clone()),
                        grantee: Some(grantee.clone()),
                        lease_state: Some(LeaseState::Available),
                    },
                )?;
            }
            Ok(id)
        })
    }

    pub fn release(&mut self, id: &str) -> Result<()> {
        self.release_for(id, &PeerIdentity::current())
    }

    pub(crate) fn release_for(&mut self, id: &str, caller: &PeerIdentity) -> Result<()> {
        self.db.transaction(|tx| {
            let Some(reference) = tx.get::<Reference>("leases", id)? else {
                return Ok(());
            };
            reference.validate_lease()?;
            match reference.lease_state {
                Some(LeaseState::Available) => ensure!(
                    reference.owner.as_ref() == Some(caller)
                        || reference.grantee.as_ref() == Some(caller),
                    "lease release unauthorized"
                ),
                Some(LeaseState::Claimed) => ensure!(
                    reference.grantee.as_ref() == Some(caller),
                    "claimed lease release unauthorized"
                ),
                None => unreachable!(),
            }
            tx.remove("leases", id)
        })
    }

    pub fn leased(&self, id: &str, digest: &ArtifactDigest) -> Result<bool> {
        self.leased_for(id, digest, &PeerIdentity::current())
    }

    pub(crate) fn leased_for(
        &self,
        id: &str,
        digest: &ArtifactDigest,
        caller: &PeerIdentity,
    ) -> Result<bool> {
        self.db.transaction(|tx| {
            let Some(reference) = tx.get::<Reference>("leases", id)? else {
                return Ok(false);
            };
            reference.authorize_grantee(caller)?;
            reachable_tx(tx, &reference.digest, digest, self.limits.max_graph)
        })
    }

    pub(crate) fn claim_lease_for(
        &self,
        id: &str,
        digest: &ArtifactDigest,
        caller: &PeerIdentity,
    ) -> Result<bool> {
        self.db.transaction(|tx| {
            let mut reference = tx
                .get::<Reference>("leases", id)?
                .ok_or_else(|| anyhow::anyhow!("lease not found"))?;
            reference.authorize_grantee(caller)?;
            ensure!(
                reachable_tx(tx, &reference.digest, digest, self.limits.max_graph)?,
                "lease does not cover digest"
            );
            match reference.lease_state {
                Some(LeaseState::Available) => {
                    reference.lease_state = Some(LeaseState::Claimed);
                    tx.put("leases", id, &reference)?;
                    Ok(true)
                }
                Some(LeaseState::Claimed) => Ok(true),
                None => unreachable!(),
            }
        })
    }
}

fn validate_epoch(value: &str) -> Result<()> {
    ensure!(
        value.len() == 32
            && value
                .bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase()),
        "invalid local lease epoch"
    );
    Ok(())
}

fn reachable_tx(
    tx: &mut StateTx,
    root: &ArtifactDigest,
    wanted: &ArtifactDigest,
    limit: usize,
) -> Result<bool> {
    let mut seen = BTreeSet::new();
    let mut pending = vec![root.clone()];
    while let Some(current) = pending.pop() {
        if current == *wanted {
            return Ok(true);
        }
        if !seen.insert(current.clone()) {
            continue;
        }
        ensure!(seen.len() <= limit, "reachability graph exceeds bound");
        if let Some(children) = tx.get::<Vec<ArtifactDigest>>("edges", current.as_str())? {
            pending.extend(children);
        }
    }
    Ok(false)
}
