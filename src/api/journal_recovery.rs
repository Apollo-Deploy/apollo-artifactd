//! Bounded semantic audit and explicit terminal outcomes for interrupted calls.
use super::journal::{KEY, load};
use crate::{Store, state::Operation};
use anyhow::{Result, ensure};
use artifactd_protocol::{Action, OperationId};
use serde_json::{Value, json};

pub(super) fn interrupted_result() -> Result<Value, String> {
    Err(
        "operation interrupted; outcome uncertain; inspect state before retrying with a new token"
            .into(),
    )
}

pub(super) fn audit(store: &Store, recover: bool) -> Result<Value> {
    store.db.transaction(|tx| {
        let journal = load(tx)?;
        let mut sequence = journal.retired_through + 1;
        let mut registry_count = 0u64;
        let mut interrupted = 0u64;
        let mut after = None;
        loop {
            let page = tx.scan::<Operation>("operations", after.as_deref(), 64)?;
            if page.is_empty() {
                break;
            }
            for (key, mut record) in page {
                ensure!(
                    record.owner.is_some(),
                    "legacy operation ownership requires explicit migration"
                );
                let expected =
                    OperationId::token(&journal.epoch, sequence).map_err(anyhow::Error::msg)?;
                ensure!(
                    key == expected.as_str() && record.sequence == sequence,
                    "corrupt operation journal sequence"
                );
                let registry = if record.phase == "allocated" {
                    ensure!(
                        record.request.is_empty() && record.result.is_none(),
                        "corrupt operation reservation"
                    );
                    false
                } else {
                    let action: Action = serde_json::from_str(&record.request)?;
                    ensure!(action.is_mutation(), "corrupt operation action");
                    match record.phase.as_str() {
                        "intent" => ensure!(record.result.is_none(), "corrupt operation intent"),
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
                    matches!(action, Action::Pull { .. } | Action::Push { .. })
                };
                let companion = tx.get::<Operation>("registry", &key)?;
                ensure!(
                    if registry {
                        companion.as_ref() == Some(&record)
                    } else {
                        companion.is_none()
                    },
                    "corrupt registry journal relationship"
                );
                registry_count += u64::from(registry);
                if record.phase == "intent" {
                    interrupted += 1;
                    if recover {
                        // Effects may have completed before a lost completion commit.
                        // Never claim success or repeat an unknown remote effect.
                        let outcome = interrupted_result();
                        record.phase = "failed".into();
                        record.result = Some(serde_json::to_string(&outcome)?);
                        tx.put("operations", &key, &record)?;
                        if registry {
                            tx.put("registry", &key, &record)?;
                        }
                    }
                }
                sequence += 1;
                after = Some(key);
            }
        }
        ensure!(
            sequence == journal.next_sequence,
            "corrupt operation journal range"
        );
        ensure!(
            tx.count("registry")? == registry_count,
            "foreign registry journal record"
        );
        if recover {
            tx.put("metadata", KEY, &journal)?;
        }
        Ok(
            json!({"verified":true,"retained":sequence-journal.retired_through-1,
            "retired_through":journal.retired_through,
            "interrupted":if recover { interrupted } else { 0 },
            "pending_intents":if recover { 0 } else { interrupted }}),
        )
    })
}
