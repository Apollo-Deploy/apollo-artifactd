//! Bounded replay with daemon-issued identities and a durable retirement floor.
use crate::{
    Store,
    state::{Operation, PeerIdentity, StateTx},
};
use anyhow::{Result, ensure};
use artifactd_protocol::OperationId;
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const WINDOW: u64 = 4096;
const RETIRE_BATCH: u64 = 64;
pub(super) const KEY: &str = "operation_journal_v2";

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Journal {
    pub epoch: String,
    pub next_sequence: u64,
    pub retired_through: u64,
}

pub(super) fn load(tx: &mut StateTx) -> Result<Journal> {
    let journal = match tx.get::<Journal>("metadata", KEY)? {
        Some(journal) => journal,
        None => {
            // Unknown old IDs cannot be pruned without losing retry safety.
            // Keep legacy journals intact rather than silently accepting them.
            ensure!(
                tx.count("operations")? == 0 && tx.count("registry")? == 0,
                "legacy operation journal requires migration"
            );
            Journal {
                epoch: uuid::Uuid::new_v4().simple().to_string(),
                next_sequence: 1,
                retired_through: 0,
            }
        }
    };
    OperationId::token(&journal.epoch, 1).map_err(anyhow::Error::msg)?;
    ensure!(
        journal.next_sequence > journal.retired_through && journal.next_sequence > 0,
        "corrupt operation journal bounds"
    );
    let retained = journal.next_sequence - journal.retired_through - 1;
    ensure!(
        retained <= WINDOW && tx.count("operations")? == retained,
        "corrupt operation journal inventory"
    );
    ensure!(
        tx.count("registry")? <= retained,
        "corrupt registry journal inventory"
    );
    Ok(journal)
}

fn validate(journal: &Journal, id: &OperationId) -> Result<u64> {
    let (epoch, sequence) = id.token_parts().map_err(anyhow::Error::msg)?;
    ensure!(
        epoch == journal.epoch,
        "operation belongs to another journal epoch"
    );
    ensure!(sequence > journal.retired_through, "operation expired");
    ensure!(
        sequence < journal.next_sequence,
        "operation was not allocated"
    );
    Ok(sequence)
}

fn get(tx: &mut StateTx, id: &OperationId, sequence: u64) -> Result<Operation> {
    let record = tx
        .get::<Operation>("operations", id.as_str())?
        .ok_or_else(|| anyhow::anyhow!("missing allocated operation"))?;
    ensure!(record.sequence == sequence, "corrupt operation sequence");
    Ok(record)
}

pub(super) fn allocate(store: &Store, caller: &PeerIdentity) -> Result<OperationId> {
    // Requests execute serially. Any remaining intent between calls has an
    // unknown outcome, rather than a concurrently running owner. Resolve it
    // explicitly before retirement so a failed completion commit cannot wedge
    // the live daemon until restart.
    if let Some(journal) = store.db.get::<Journal>("metadata", KEY)?
        && journal.next_sequence.checked_sub(journal.retired_through) == Some(WINDOW + 1)
    {
        super::journal_recovery::audit(store, true)?;
    }
    store.db.transaction(|tx| {
        let mut journal = load(tx)?;
        let retained = journal.next_sequence - journal.retired_through - 1;
        if retained == WINDOW {
            for _ in 0..RETIRE_BATCH {
                let sequence = journal.retired_through + 1;
                let id =
                    OperationId::token(&journal.epoch, sequence).map_err(anyhow::Error::msg)?;
                let record = get(tx, &id, sequence)?;
                match record.phase.as_str() {
                    "intent" => break,
                    "allocated" => ensure!(
                        record.request.is_empty() && record.result.is_none(),
                        "corrupt operation reservation"
                    ),
                    "complete" | "failed" => {
                        let result: Result<Value, String> = serde_json::from_str(
                            record
                                .result
                                .as_deref()
                                .ok_or_else(|| anyhow::anyhow!("missing operation result"))?,
                        )?;
                        ensure!(
                            result.is_ok() == (record.phase == "complete"),
                            "corrupt operation completion"
                        );
                    }
                    _ => anyhow::bail!("corrupt operation phase"),
                }
                tx.remove("operations", id.as_str())?;
                tx.remove("registry", id.as_str())?;
                journal.retired_through = sequence;
            }
        }
        ensure!(
            journal.next_sequence - journal.retired_through - 1 < WINDOW,
            "operation window blocked by unresolved intent; retry its token"
        );
        let id = OperationId::token(&journal.epoch, journal.next_sequence)
            .map_err(anyhow::Error::msg)?;
        tx.put(
            "operations",
            id.as_str(),
            &Operation {
                request: String::new(),
                phase: "allocated".to_owned(),
                result: None,
                sequence: journal.next_sequence,
                owner: Some(caller.clone()),
            },
        )?;
        journal.next_sequence = journal
            .next_sequence
            .checked_add(1)
            .ok_or_else(|| anyhow::anyhow!("operation sequence exhausted"))?;
        tx.put("metadata", KEY, &journal)?;
        Ok(id)
    })
}

/// Returns a terminal result for replay, or persists intent before any effect.
pub(super) fn begin(
    store: &Store,
    id: &OperationId,
    payload: &str,
    registry: bool,
    caller: &PeerIdentity,
) -> Result<Option<Result<Value, String>>> {
    store.db.transaction(|tx| {
        let journal = load(tx)?;
        let sequence = validate(&journal, id)?;
        let mut record = get(tx, id, sequence)?;
        ensure!(
            record.owner.as_ref() == Some(caller),
            "operation owner mismatch"
        );
        match record.phase.as_str() {
            "allocated" => {
                ensure!(
                    record.request.is_empty() && record.result.is_none(),
                    "corrupt operation reservation"
                );
                record.request = payload.to_owned();
                record.phase = "intent".to_owned();
                tx.put("operations", id.as_str(), &record)?;
                if registry {
                    tx.put("registry", id.as_str(), &record)?;
                }
                Ok(None)
            }
            "intent" | "complete" | "failed" => {
                ensure!(record.request == payload, "operation identity conflict");
                if record.phase == "intent" {
                    ensure!(record.result.is_none(), "corrupt operation intent");
                    let result = super::journal_recovery::interrupted_result();
                    record.phase = "failed".into();
                    record.result = Some(serde_json::to_string(&result)?);
                    tx.put("operations", id.as_str(), &record)?;
                    if registry {
                        tx.put("registry", id.as_str(), &record)?;
                    }
                    Ok(Some(result))
                } else {
                    let result: Result<Value, String> = serde_json::from_str(
                        record
                            .result
                            .as_deref()
                            .ok_or_else(|| anyhow::anyhow!("missing operation result"))?,
                    )?;
                    ensure!(
                        result.is_ok() == (record.phase == "complete"),
                        "corrupt operation completion"
                    );
                    Ok(Some(result))
                }
            }
            _ => anyhow::bail!("corrupt operation phase"),
        }
    })
}

pub(super) fn complete(
    store: &Store,
    id: &OperationId,
    payload: &str,
    outcome: &Result<&Value, String>,
    registry: bool,
    caller: &PeerIdentity,
) -> Result<()> {
    store.db.transaction(|tx| {
        let journal = load(tx)?;
        let sequence = validate(&journal, id)?;
        let mut record = get(tx, id, sequence)?;
        ensure!(
            record.owner.as_ref() == Some(caller),
            "operation owner mismatch"
        );
        ensure!(
            record.phase == "intent" && record.request == payload,
            "operation completion lost its intent"
        );
        record.phase = if outcome.is_ok() {
            "complete"
        } else {
            "failed"
        }
        .to_owned();
        record.result = Some(serde_json::to_string(outcome)?);
        tx.put("operations", id.as_str(), &record)?;
        if registry {
            tx.put("registry", id.as_str(), &record)?;
        }
        Ok(())
    })
}
