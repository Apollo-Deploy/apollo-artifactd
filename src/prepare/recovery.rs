use super::tree;
use crate::{Store, filesystem};
use anyhow::{Result, ensure};

impl Store {
    pub(crate) fn recover_prepared(&mut self, max: u32) -> Result<()> {
        let entries = {
            let mut q = self.db.prepare("SELECT id,staging,tree_digest,phase FROM prepared WHERE phase!='complete' LIMIT ?1")?;
            q.query_map([max], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, Option<String>>(2)?,
                    r.get::<_, String>(3)?,
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?
        };
        for (id, staging, expected, phase) in entries {
            ensure!(
                id.len() == 64
                    && id
                        .bytes()
                        .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)),
                "corrupt prepared identity"
            );
            let token = staging
                .strip_suffix(".staging")
                .ok_or_else(|| anyhow::anyhow!("invalid staging record"))?;
            uuid::Uuid::parse_str(token)?;
            if phase == "deleting" || phase == "gc_intent" {
                if self.prepared.symlink_metadata(&id).is_ok() {
                    let dir = self.prepared.open_dir(&id)?;
                    if phase == "gc_intent" {
                        ensure!(
                            tree::verify(&dir)?
                                == expected
                                    .ok_or_else(|| anyhow::anyhow!("missing prepared hash"))?,
                            "prepared deletion integrity mismatch"
                        );
                        self.db
                            .execute("UPDATE prepared SET phase='deleting' WHERE id=?1", [&id])?;
                    }
                    tree::remove(&self.prepared, &id)?;
                }
            } else if self.prepared.symlink_metadata(&id).is_ok() {
                let dir = self.prepared.open_dir(&id)?;
                ensure!(
                    tree::verify(&dir)?
                        == expected
                            .ok_or_else(|| anyhow::anyhow!("unproven prepared publication"))?,
                    "prepared publication integrity mismatch"
                );
                filesystem::sync(&self.prepared)?;
                self.db
                    .execute("UPDATE prepared SET phase='complete' WHERE id=?1", [&id])?;
                continue;
            }
            if self.prepared.symlink_metadata(&staging).is_ok() {
                tree::remove(&self.prepared, &staging)?;
            }
            filesystem::sync(&self.prepared)?;
            self.db.execute("DELETE FROM prepared WHERE id=?1", [&id])?;
        }
        Ok(())
    }

    pub(crate) fn gc_prepared(&mut self, max: u32) -> Result<u32> {
        let ids = {
            let mut q = self.db.prepare("WITH RECURSIVE live(digest) AS (SELECT digest FROM pins UNION SELECT digest FROM leases UNION SELECT edges.child FROM edges JOIN live ON edges.parent=live.digest) SELECT id FROM prepared WHERE phase='complete' AND manifest NOT IN (SELECT digest FROM live) LIMIT ?1")?;
            q.query_map([max], |r| r.get::<_, String>(0))?
                .collect::<Result<Vec<_>, _>>()?
        };
        let tx = self.db.transaction()?;
        for id in &ids {
            tx.execute("UPDATE prepared SET phase='gc_intent' WHERE id=?1", [id])?;
        }
        tx.commit()?;
        self.recover_prepared(max)?;
        Ok(ids.len() as u32)
    }
}
