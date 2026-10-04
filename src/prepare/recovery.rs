use super::tree;
use crate::{Store, filesystem};
use anyhow::{Result, ensure};

impl Store {
    pub(crate) fn recover_prepared(&mut self, max: u32) -> Result<()> {
        ensure!(max > 0 && max <= 4096, "invalid prepared recovery bound");
        let cursor = self
            .db
            .get::<String>("metadata", "prepared_recovery_cursor")?;
        let mut entries =
            self.db
                .scan::<crate::state::Prepared>("prepared", cursor.as_deref(), max as usize)?;
        if entries.is_empty() && cursor.is_some() {
            self.db.remove("metadata", "prepared_recovery_cursor")?;
            entries = self
                .db
                .scan::<crate::state::Prepared>("prepared", None, max as usize)?;
        }
        let next = entries.last().map(|(id, _)| id.clone());
        for (id, record) in entries {
            self.recover_prepared_record(&id, &record)?;
        }
        if let Some(next) = next {
            self.db.put("metadata", "prepared_recovery_cursor", &next)?;
        } else if cursor.is_some() {
            self.db.remove("metadata", "prepared_recovery_cursor")?;
        }
        Ok(())
    }

    pub(crate) fn recover_prepared_id(&mut self, id: &str) -> Result<()> {
        if let Some(record) = self.db.get::<crate::state::Prepared>("prepared", id)? {
            self.recover_prepared_record(id, &record)?;
        }
        Ok(())
    }

    fn recover_prepared_record(&mut self, id: &str, record: &crate::state::Prepared) -> Result<()> {
        ensure!(
            id.len() == 64 && id.bytes().all(|b| b.is_ascii_hexdigit()),
            "corrupt prepared identity"
        );
        let token = record
            .staging
            .strip_suffix(".staging")
            .ok_or_else(|| anyhow::anyhow!("invalid staging record"))?;
        uuid::Uuid::parse_str(token)?;
        if record.phase == "deleting" || record.phase == "gc_intent" {
            if !self.gc_snapshot_ready()? {
                return Ok(());
            }
            ensure!(
                !self.gc_marked(&record.manifest, true)?,
                "prepared GC intent conflicts with active handles"
            );
            if self.prepared.symlink_metadata(id).is_ok() {
                tree::remove(&self.prepared, id)?;
            }
        } else if self.prepared.symlink_metadata(id).is_ok() {
            let expected = record
                .tree_digest
                .clone()
                .ok_or_else(|| anyhow::anyhow!("unproven prepared publication"))?;
            let dir = self.prepared.open_dir(id)?;
            ensure!(
                tree::verify(&dir)? == expected,
                "prepared publication integrity mismatch"
            );
            if record.phase != "complete" {
                let mut updated = record.clone();
                updated.phase = "complete".to_owned();
                self.db.put("prepared", id, &updated)?;
            }
            return Ok(());
        }
        if self.prepared.symlink_metadata(&record.staging).is_ok() {
            tree::remove(&self.prepared, &record.staging)?;
        }
        filesystem::sync(&self.prepared)?;
        self.gc_complete_prepared(id, record)?;
        Ok(())
    }

    pub(crate) fn gc_prepared(&mut self, max: u32) -> Result<u32> {
        let mut ids = Vec::new();
        ensure!(max > 0 && max <= 4096, "invalid prepared gc bound");
        let cursor = self.db.get::<String>("metadata", "prepared_gc_cursor")?;
        let mut entries =
            self.db
                .scan::<crate::state::Prepared>("prepared", cursor.as_deref(), max as usize)?;
        if entries.is_empty() && cursor.is_some() {
            self.db.remove("metadata", "prepared_gc_cursor")?;
            entries = self
                .db
                .scan::<crate::state::Prepared>("prepared", None, max as usize)?;
        }
        let next = entries.last().map(|(id, _)| id.clone());
        for (id, record) in entries {
            if record.phase == "complete" && !self.gc_marked(&record.manifest, true)? {
                ids.push(id);
            }
        }
        for id in &ids {
            if let Some(mut record) = self.db.get::<crate::state::Prepared>("prepared", id)? {
                record.phase = "gc_intent".to_owned();
                self.db.put("prepared", id, &record)?;
            }
            self.recover_prepared_id(id)?;
        }
        if let Some(next) = next {
            self.db.put("metadata", "prepared_gc_cursor", &next)?;
        } else if cursor.is_some() {
            self.db.remove("metadata", "prepared_gc_cursor")?;
        }
        Ok(ids.len() as u32)
    }
}
