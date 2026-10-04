//! Persisted, bounded root marking shared by collection and crash recovery.
mod prepared;
mod sweep;
use crate::{Store, state};
use anyhow::{Result, ensure};
use artifactd_protocol::ArtifactDigest;
use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize)]
enum Phase {
    Clear,
    Pins,
    Leases,
    Prepared,
    Ready,
}

#[derive(Serialize, Deserialize)]
struct Cycle {
    epoch: u64,
    phase: Phase,
    cursor: Option<String>,
    scanned: u64,
}

#[derive(Serialize, Deserialize)]
struct Mark {
    epoch: u64,
    handles: bool,
    prepared: u64,
}

impl Cycle {
    fn new(epoch: u64) -> Self {
        Self {
            epoch,
            phase: Phase::Clear,
            cursor: None,
            scanned: 0,
        }
    }
}

impl Store {
    pub(crate) fn gc_reset(&self) -> Result<()> {
        self.db.remove("metadata", "gc_cycle")
    }

    fn reference_epoch(&self) -> Result<u64> {
        self.db
            .get("metadata", "gc_reference_epoch")?
            .ok_or_else(|| anyhow::anyhow!("missing GC reference epoch"))
    }

    pub(crate) fn gc_snapshot_ready(&self) -> Result<bool> {
        let Some(cycle) = self.db.get::<Cycle>("metadata", "gc_cycle")? else {
            return Ok(false);
        };
        Ok(matches!(cycle.phase, Phase::Ready) && cycle.epoch == self.reference_epoch()?)
    }

    /// At most 64 root records are verified per invocation. Each graph has
    /// independent descriptor, metadata, byte, and node bounds. No candidate
    /// is deleted until every live root in the unchanged epoch is verified.
    pub(crate) fn gc_ready(&self, max: u32) -> Result<bool> {
        ensure!((1..=4096).contains(&max), "invalid GC mark bound");
        let epoch = self.reference_epoch()?;
        let mut cycle = self
            .db
            .get::<Cycle>("metadata", "gc_cycle")?
            .unwrap_or_else(|| Cycle::new(epoch));
        if cycle.epoch != epoch {
            cycle = Cycle::new(epoch);
        }
        let mut budget = max.min(64) as usize;
        if matches!(cycle.phase, Phase::Clear) {
            let page = self.db.scan::<Mark>("gc_marks", None, budget)?;
            self.db.transaction(|tx| {
                for (key, _) in page {
                    tx.remove("gc_marks", &key)?;
                }
                Ok(())
            })?;
            if self.db.count("gc_marks")? != 0 {
                self.db.put("metadata", "gc_cycle", &cycle)?;
                return Ok(false);
            }
            cycle.phase = Phase::Pins;
        }
        loop {
            let (table, next, handles) = match cycle.phase {
                Phase::Pins => ("pins", Phase::Leases, true),
                Phase::Leases => ("leases", Phase::Prepared, true),
                Phase::Prepared => ("prepared", Phase::Ready, false),
                Phase::Ready => {
                    ensure!(cycle.epoch == self.reference_epoch()?, "GC epoch changed");
                    self.db.put("metadata", "gc_cycle", &cycle)?;
                    return Ok(true);
                }
                Phase::Clear => unreachable!(),
            };
            // The inventory can shrink during ordinary preparation recovery.
            // A global count compared with an old scanned count can skip a
            // surviving suffix; only the cursor's actual range proves EOF.
            let exhausted = if handles {
                self.db
                    .scan::<state::Reference>(table, cycle.cursor.as_deref(), 1)?
                    .is_empty()
            } else {
                self.db
                    .scan::<state::Prepared>(table, cycle.cursor.as_deref(), 1)?
                    .is_empty()
            };
            if exhausted {
                cycle.phase = next;
                cycle.cursor = None;
                cycle.scanned = 0;
                continue;
            }
            if budget == 0 {
                self.db.put("metadata", "gc_cycle", &cycle)?;
                return Ok(false);
            }
            let roots = if handles {
                self.db
                    .scan::<state::Reference>(table, cycle.cursor.as_deref(), budget)?
                    .into_iter()
                    .map(|(key, record)| (key, Some(record.digest)))
                    .collect::<Vec<_>>()
            } else {
                self.db
                    .scan::<state::Prepared>(table, cycle.cursor.as_deref(), budget)?
                    .into_iter()
                    .map(|(key, record)| {
                        (
                            key,
                            matches!(record.phase.as_str(), "complete" | "gc_intent" | "deleting")
                                .then_some(record.manifest),
                        )
                    })
                    .collect::<Vec<_>>()
            };
            ensure!(
                !roots.is_empty(),
                "GC root cursor inconsistent with inventory"
            );
            for (key, root) in roots {
                cycle.cursor = Some(key);
                cycle.scanned = cycle
                    .scanned
                    .checked_add(1)
                    .ok_or_else(|| anyhow::anyhow!("GC root count overflow"))?;
                self.gc_mark_root(root.as_ref(), &cycle, handles)?;
                budget -= 1;
            }
            self.db.put("metadata", "gc_cycle", &cycle)?;
        }
    }

    fn gc_mark_root(
        &self,
        digest: Option<&ArtifactDigest>,
        cycle: &Cycle,
        handles: bool,
    ) -> Result<()> {
        let nodes = digest
            .map(|d| self.verified_graph_nodes(d))
            .transpose()?
            .unwrap_or_default();
        self.db.transaction(|tx| {
            for node in nodes {
                let previous = tx.get::<Mark>("gc_marks", node.as_str())?;
                ensure!(
                    previous.as_ref().is_none_or(|m| m.epoch == cycle.epoch),
                    "GC mark epoch mismatch"
                );
                tx.put(
                    "gc_marks",
                    node.as_str(),
                    &Mark {
                        epoch: cycle.epoch,
                        handles: handles || previous.as_ref().is_some_and(|m| m.handles),
                        prepared: previous
                            .map_or(0, |m| m.prepared)
                            .checked_add(u64::from(!handles))
                            .ok_or_else(|| anyhow::anyhow!("GC prepared count overflow"))?,
                    },
                )?;
            }
            // Mark counts and the root cursor must commit together, so replay
            // after a crash cannot count a prepared root twice.
            tx.put("metadata", "gc_cycle", cycle)?;
            Ok(())
        })
    }

    pub(crate) fn gc_marked(&self, digest: &ArtifactDigest, handles: bool) -> Result<bool> {
        let cycle = self
            .db
            .get::<Cycle>("metadata", "gc_cycle")?
            .ok_or_else(|| anyhow::anyhow!("GC marks not ready"))?;
        ensure!(
            matches!(cycle.phase, Phase::Ready) && cycle.epoch == self.reference_epoch()?,
            "GC protection snapshot is stale"
        );
        let mark = self.db.get::<Mark>("gc_marks", digest.as_str())?;
        if let Some(mark) = mark {
            ensure!(
                mark.epoch == cycle.epoch,
                "GC mark belongs to another epoch"
            );
            Ok(mark.handles || (!handles && mark.prepared > 0))
        } else {
            Ok(false)
        }
    }
}
