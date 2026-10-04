use super::Store;
use crate::filesystem;
use anyhow::{Result, ensure};
use artifactd_protocol::ArtifactDigest;
use cap_std::fs::MetadataExt;

impl Store {
    pub fn reconcile(&mut self, max: u32) -> Result<u32> {
        ensure!(max > 0 && max <= 4096, "invalid reconcile bound");
        let imports = {
            let mut q = self
                .db
                .prepare("SELECT id,temp,digest,size FROM imports ORDER BY id LIMIT ?1")?;
            q.query_map([max], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    crate::state::unsigned(r, 3)?,
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?
        };
        let mut recovered = 0;
        for (id, temp, digest, size) in imports {
            let digest: ArtifactDigest = digest.parse()?;
            ensure!(
                uuid::Uuid::parse_str(&id).is_ok() && temp == format!("{id}.part"),
                "corrupt import ownership record"
            );
            if let Ok(meta) = self.temp.symlink_metadata(&temp) {
                if meta.nlink() == 2 {
                    let final_meta = self.blobs.symlink_metadata(digest.hex())?;
                    ensure!(
                        meta.dev() == final_meta.dev()
                            && meta.ino() == final_meta.ino()
                            && meta.is_file()
                            && meta.uid() == rustix::process::geteuid().as_raw()
                            && meta.mode() & 0o277 == 0,
                        "unproven publication ownership"
                    );
                    self.temp.remove_file(&temp)?;
                    filesystem::sync(&self.temp)?;
                } else {
                    filesystem::owned_remove(&self.temp, &temp)?;
                }
            }
            if self.blobs.symlink_metadata(digest.hex()).is_ok() {
                self.verify_file(&digest, size)?;
                filesystem::sync(&self.blobs)?;
                self.db.execute(
                    "INSERT INTO blobs(digest,size) VALUES (?1,?2) ON CONFLICT(digest) DO NOTHING",
                    rusqlite::params![digest.as_str(), i64::try_from(size)?],
                )?;
            }
            self.db.execute("DELETE FROM imports WHERE id=?1", [&id])?;
            recovered += 1;
        }
        self.recover_prepared(max)?;
        self.recover_gc(max)?;
        Ok(recovered)
    }

    pub(crate) fn recover_gc(&mut self, max: u32) -> Result<()> {
        self.verify_live_graphs()?;
        let entries = {
            let mut q = self.db.prepare("SELECT digest,size FROM gc LIMIT ?1")?;
            q.query_map([max], |r| {
                Ok((r.get::<_, String>(0)?, crate::state::unsigned(r, 1)?))
            })?
            .collect::<Result<Vec<_>, _>>()?
        };
        for (digest, size) in entries {
            let d: ArtifactDigest = digest.parse()?;
            ensure!(
                !self.protected(&d)?,
                "gc intent conflicts with protected content"
            );
            match self.blobs.symlink_metadata(d.hex()) {
                Ok(_) => {
                    self.verify_file(&d, size)?;
                    filesystem::owned_remove(&self.blobs, d.hex())?;
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    filesystem::sync(&self.blobs)?;
                }
                Err(e) => return Err(e.into()),
            }
            let tx = self.db.transaction()?;
            tx.execute("DELETE FROM edges WHERE parent=?1 OR child=?1", [&digest])?;
            tx.execute("DELETE FROM roots WHERE digest=?1", [&digest])?;
            tx.execute("DELETE FROM blobs WHERE digest=?1", [&digest])?;
            tx.execute("DELETE FROM gc WHERE digest=?1", [&digest])?;
            tx.commit()?;
        }
        Ok(())
    }
}
