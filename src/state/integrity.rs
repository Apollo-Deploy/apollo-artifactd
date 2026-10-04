use super::State;
use crate::filesystem;
use anyhow::{Context, Result};

impl State {
    /// Checks and, when possible, repairs redb's page and allocator state.
    /// A repaired result is deliberately reported as non-clean to callers.
    pub fn check_integrity(&mut self) -> Result<bool> {
        self.require_healthy()?;
        self.healthy = false;
        let result = self.db.check_integrity().context("redb integrity check");
        match result {
            Ok(clean) => {
                filesystem::sync(&self.root).context("sync repaired redb state")?;
                if clean {
                    self.healthy = true;
                }
                Ok(clean)
            }
            Err(error) => Err(error),
        }
    }
}
