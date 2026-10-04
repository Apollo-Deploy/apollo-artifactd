use super::Store;
use crate::filesystem;
use anyhow::{Result, ensure};
use artifactd_protocol::ArtifactDigest;
use cap_std::fs::MetadataExt;

impl Store {
    pub fn reconcile(&mut self, max: u32) -> Result<u32> {
        ensure!(max > 0 && max <= 4096, "invalid reconcile bound");
        let imports = self
            .db
            .scan::<crate::state::Imported>("imports", None, max as usize)?;
        let mut recovered = 0;
        for (id, record) in imports {
            ensure!(
                uuid::Uuid::parse_str(&id).is_ok() && record.temp == format!("{id}.part"),
                "corrupt import ownership record"
            );
            let Some(digest) = record.digest else {
                if self.temp.symlink_metadata(&record.temp).is_ok() {
                    filesystem::owned_remove(&self.temp, &record.temp)?;
                }
                self.db.remove("imports", &id)?;
                recovered += 1;
                continue;
            };
            if let Ok(meta) = self.temp.symlink_metadata(&record.temp) {
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
                    self.temp.remove_file(&record.temp)?;
                    filesystem::sync(&self.temp)?;
                } else {
                    filesystem::owned_remove(&self.temp, &record.temp)?;
                }
            }
            if self.blobs.symlink_metadata(digest.hex()).is_ok() {
                self.verify_file(&digest, record.size)?;
                filesystem::sync(&self.blobs)?;
                self.db.put(
                    "blobs",
                    digest.as_str(),
                    &crate::state::Blob { size: record.size },
                )?;
            }
            self.db.remove("imports", &id)?;
            recovered += 1;
        }
        self.recover_prepared(max)?;
        self.recover_gc(max)?;
        Ok(recovered)
    }

    pub(crate) fn recover_gc(&mut self, max: u32) -> Result<()> {
        self.verify_live_graphs()?;
        let entries = self
            .db
            .scan::<crate::state::Garbage>("gc", None, max as usize)?;
        for (digest, garbage) in entries {
            let digest: ArtifactDigest = digest.parse()?;
            ensure!(
                !self.protected(&digest)?,
                "gc intent conflicts with protected content"
            );
            match self.blobs.symlink_metadata(digest.hex()) {
                Ok(_) => {
                    self.verify_file(&digest, garbage.size)?;
                    filesystem::owned_remove(&self.blobs, digest.hex())?;
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    filesystem::sync(&self.blobs)?
                }
                Err(e) => return Err(e.into()),
            }
            self.db.transaction(|tx| {
                tx.remove("edges", digest.as_str())?;
                // Surviving parents still describe these immutable bytes.
                // Keep their incoming edges: a missing child fails graph
                // verification, and exact leaf reimport restores that graph.
                // Collecting a parent removes its own outgoing edges without
                // a global scan or rewriting unrelated descriptor relations.
                tx.remove("roots", digest.as_str())?;
                tx.remove("blobs", digest.as_str())?;
                tx.remove("gc", digest.as_str())
            })?;
        }
        Ok(())
    }
}
