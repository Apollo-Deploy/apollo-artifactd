//! Persisted, bounded root marking shared by collection and crash recovery.
mod prepared;
mod sweep;
pub(crate) mod walk;
use crate::{Store, state};
use anyhow::{Result, ensure};
use artifactd_protocol::ArtifactDigest;
use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
enum Phase {
    Clear,
    Pins,
    Leases,
    Prepared,
    Ready,
}

#[derive(Clone, Serialize, Deserialize)]
struct Cycle {
    epoch: u64,
    phase: Phase,
    cursor: Option<String>,
    scanned: u64,
    #[serde(default)]
    walk: Option<walk::GcWalk>,
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
            walk: None,
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

    /// Performs a bounded number of root and metadata work units per call.
    /// OCI graph discovery, persisted-edge comparison, and classification
    /// checks resume from the durable cursor. No marks publish before proof is
    /// complete for the whole root.
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
        let mut work = walk::Budget::new();
        let mut roots_left = max.min(64) as usize;
        if matches!(cycle.phase, Phase::Clear) {
            let page = self.db.scan::<Mark>("gc_marks", None, roots_left)?;
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
            if let Some(walk) = cycle.walk.as_mut() {
                if !self.gc_root_still_valid(&cycle.phase, walk)? {
                    cycle.walk = None;
                    self.db.put("metadata", "gc_cycle", &cycle)?;
                    continue;
                }
                if !self.gc_advance_walk(walk, &mut work)? {
                    self.db.put("metadata", "gc_cycle", &cycle)?;
                    return Ok(false);
                }
                if !self.gc_finish_root(&mut cycle, &mut work)? {
                    self.db.put("metadata", "gc_cycle", &cycle)?;
                    return Ok(false);
                }
                roots_left -= 1;
                continue;
            }

            let (table, next_phase, handles) = match cycle.phase {
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
            if let Some((key, digest)) = self.gc_next_root(table, cycle.cursor.as_deref())? {
                if roots_left == 0 {
                    self.db.put("metadata", "gc_cycle", &cycle)?;
                    return Ok(false);
                }
                cycle.walk = Some(walk::GcWalk::new(key, digest, handles));
                self.db.put("metadata", "gc_cycle", &cycle)?;
                continue;
            }
            cycle.phase = next_phase;
            cycle.cursor = None;
            cycle.scanned = 0;
        }
    }

    fn gc_next_root(
        &self,
        table: &'static str,
        cursor: Option<&str>,
    ) -> Result<Option<(String, Option<ArtifactDigest>)>> {
        if table == "prepared" {
            return Ok(self
                .db
                .scan::<state::Prepared>(table, cursor, 1)?
                .into_iter()
                .next()
                .map(|(key, record)| {
                    let digest =
                        matches!(record.phase.as_str(), "complete" | "gc_intent" | "deleting")
                            .then_some(record.manifest);
                    (key, digest)
                }));
        }
        let Some((key, record)) = self
            .db
            .scan::<state::Reference>(table, cursor, 1)?
            .into_iter()
            .next()
        else {
            return Ok(None);
        };
        if table == "pins" {
            record.validate_pin()?;
        } else {
            record.validate_lease()?;
        }
        Ok(Some((key, Some(record.digest))))
    }

    fn gc_root_still_valid(&self, phase: &Phase, walk: &walk::GcWalk) -> Result<bool> {
        let table = match phase {
            Phase::Pins => "pins",
            Phase::Leases => "leases",
            Phase::Prepared => "prepared",
            _ => return Ok(false),
        };
        if table == "prepared" {
            let Some(record) = self.db.get::<state::Prepared>(table, &walk.key)? else {
                return Ok(false);
            };
            let active = matches!(record.phase.as_str(), "complete" | "gc_intent" | "deleting");
            return Ok(walk.digest.as_ref() == active.then_some(&record.manifest));
        }
        let Some(record) = self.db.get::<state::Reference>(table, &walk.key)? else {
            return Ok(false);
        };
        if table == "pins" {
            record.validate_pin()?;
        } else {
            record.validate_lease()?;
        }
        Ok(walk.digest.as_ref() == Some(&record.digest))
    }

    fn gc_finish_root(&self, cycle: &mut Cycle, work: &mut walk::Budget) -> Result<bool> {
        let walk = cycle
            .walk
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("missing GC walk"))?;
        ensure!(self.gc_walk_complete(walk), "GC graph proof is incomplete");
        let nodes = self.gc_walk_nodes(walk);
        if !work.marks(nodes.len()) {
            return Ok(false);
        }
        let mut completed = cycle.clone();
        completed.cursor = Some(walk.key.clone());
        completed.scanned = completed
            .scanned
            .checked_add(1)
            .ok_or_else(|| anyhow::anyhow!("GC root count overflow"))?;
        completed.walk = None;
        let key = walk.key.clone();
        let digest = walk.digest.clone();
        let handles = walk.handles;
        self.db.transaction(|tx| {
            ensure!(
                tx.get::<u64>("metadata", "gc_reference_epoch")? == Some(cycle.epoch),
                "GC epoch changed"
            );
            let current = tx
                .get::<Cycle>("metadata", "gc_cycle")?
                .ok_or_else(|| anyhow::anyhow!("GC cycle disappeared"))?;
            ensure!(
                current.epoch == cycle.epoch
                    && current.phase == cycle.phase
                    && current.cursor == cycle.cursor
                    && current
                        .walk
                        .as_ref()
                        .is_some_and(|active| active.key == key),
                "GC walk changed before publication"
            );
            match cycle.phase {
                Phase::Pins | Phase::Leases => {
                    let table = if cycle.phase == Phase::Pins {
                        "pins"
                    } else {
                        "leases"
                    };
                    let record = tx
                        .get::<state::Reference>(table, &key)?
                        .ok_or_else(|| anyhow::anyhow!("GC root disappeared"))?;
                    if table == "pins" {
                        record.validate_pin()?;
                    } else {
                        record.validate_lease()?;
                    }
                    ensure!(digest.as_ref() == Some(&record.digest), "GC root changed");
                }
                Phase::Prepared => {
                    let record = tx
                        .get::<state::Prepared>("prepared", &key)?
                        .ok_or_else(|| anyhow::anyhow!("prepared GC root disappeared"))?;
                    let active =
                        matches!(record.phase.as_str(), "complete" | "gc_intent" | "deleting");
                    ensure!(
                        digest.as_ref() == active.then_some(&record.manifest),
                        "prepared GC root changed"
                    );
                }
                _ => anyhow::bail!("invalid GC root phase"),
            }
            for node in &nodes {
                let previous = tx.get::<Mark>("gc_marks", node.as_str())?;
                ensure!(
                    previous
                        .as_ref()
                        .is_none_or(|mark| mark.epoch == cycle.epoch),
                    "GC mark epoch mismatch"
                );
                tx.put(
                    "gc_marks",
                    node.as_str(),
                    &Mark {
                        epoch: cycle.epoch,
                        handles: handles || previous.as_ref().is_some_and(|mark| mark.handles),
                        prepared: previous
                            .map_or(0, |mark| mark.prepared)
                            .checked_add(u64::from(!handles))
                            .ok_or_else(|| anyhow::anyhow!("GC prepared count overflow"))?,
                    },
                )?;
            }
            tx.put("metadata", "gc_cycle", &completed)
        })?;
        *cycle = completed;
        Ok(true)
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
