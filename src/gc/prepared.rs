use super::{Cycle, Mark, Phase};
use crate::{Store, state};
use anyhow::{Result, ensure};

impl Store {
    /// Release prepared-root protection only with deletion completion. A
    /// restart discards marks and reconstructs them from surviving records.
    pub(crate) fn gc_complete_prepared(&self, id: &str, record: &state::Prepared) -> Result<()> {
        let cycle = self.db.get::<Cycle>("metadata", "gc_cycle")?;
        let ready = cycle
            .as_ref()
            .is_some_and(|c| matches!(c.phase, Phase::Ready));
        if !ready || !matches!(record.phase.as_str(), "gc_intent" | "deleting") {
            return self.db.remove("prepared", id);
        }
        let cycle = cycle.unwrap();
        ensure!(
            cycle.epoch == self.reference_epoch()?,
            "prepared GC snapshot is stale"
        );
        let nodes = self.gc_reachable_nodes(&record.manifest)?;
        self.db.transaction(|tx| {
            for node in nodes {
                let mut mark = tx
                    .get::<Mark>("gc_marks", node.as_str())?
                    .ok_or_else(|| anyhow::anyhow!("missing prepared GC protection"))?;
                ensure!(
                    mark.epoch == cycle.epoch && mark.prepared > 0,
                    "prepared GC protection mismatch"
                );
                mark.prepared -= 1;
                if !mark.handles && mark.prepared == 0 {
                    tx.remove("gc_marks", node.as_str())?;
                } else {
                    tx.put("gc_marks", node.as_str(), &mark)?;
                }
            }
            tx.remove("prepared", id)
        })
    }
}
