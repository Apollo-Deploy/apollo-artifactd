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
            let recorded_blob = self
                .db
                .get::<crate::state::Blob>("blobs", digest.as_str())?
                .is_some_and(|blob| blob.size == record.size);
            if let Ok(meta) = self.temp.symlink_metadata(&record.temp) {
                if meta.nlink() == 2 {
                    let final_meta = self.blobs.symlink_metadata(digest.hex())?;
                    ensure!(
                        meta.dev() == final_meta.dev()
                            && meta.ino() == final_meta.ino()
                            && meta.is_file()
                            && meta.uid() == rustix::process::geteuid().as_raw()
                            && meta.mode() & 0o222 == 0
                            && meta.mode() & 0o277 == 0,
                        "unproven publication ownership"
                    );
                    self.temp.remove_file(&record.temp)?;
                    filesystem::sync(&self.temp)?;
                } else if meta.mode() & 0o222 != 0 || meta.len() != record.size {
                    // A stage which is still writable, or whose durable size
                    // is not the recorded size, cannot be a completed
                    // publication.  Remove it only through the descriptor
                    // relative ownership checks in owned_remove; this keeps
                    // foreign, linked, and symlink entries fail-closed.
                    filesystem::owned_remove(&self.temp, &record.temp)?;
                    self.db.remove("imports", &id)?;
                    recovered += 1;
                    continue;
                } else {
                    // The stage is private and readonly, so it may be the
                    // completed chmod step immediately before the import
                    // intent was advanced.  Hash it while those ownership
                    // checks are live; only a successful hash mismatch grants
                    // authority to discard it.  Open/hash failures remain
                    // fail-closed and never become cleanup authorization.
                    let mut staged = filesystem::read(&self.temp, &record.temp)?;
                    let (actual, staged_size) = super::hash(&mut staged, record.size)?;
                    if actual != digest || staged_size != record.size {
                        filesystem::owned_remove(&self.temp, &record.temp)?;
                        self.db.remove("imports", &id)?;
                        recovered += 1;
                        continue;
                    }
                    if self.blobs.symlink_metadata(digest.hex()).is_ok() {
                        if self.verify_file(&digest, record.size).is_ok() {
                            filesystem::owned_remove(&self.temp, &record.temp)?;
                        } else {
                            ensure!(
                                recorded_blob,
                                "unrecorded digest collision refuses recovery replacement"
                            );
                            let _existing = filesystem::read(&self.blobs, digest.hex())?;
                            self.temp.rename(&record.temp, &self.blobs, digest.hex())?;
                            filesystem::sync(&self.temp)?;
                            filesystem::sync(&self.blobs)?;
                        }
                    } else {
                        self.temp.rename(&record.temp, &self.blobs, digest.hex())?;
                        filesystem::sync(&self.temp)?;
                        filesystem::sync(&self.blobs)?;
                    }
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
        self.gc_ready(max)?;
        self.recover_prepared(max)?;
        self.recover_gc(max)?;
        Ok(recovered)
    }

    pub(crate) fn recover_gc(&mut self, max: u32) -> Result<()> {
        if !self.gc_snapshot_ready()? {
            return Ok(());
        }
        let entries = self
            .db
            .scan::<crate::state::Garbage>("gc", None, max as usize)?;
        for (digest, garbage) in entries {
            let digest: ArtifactDigest = digest.parse()?;
            ensure!(
                !self.gc_marked(&digest, false)?,
                "gc intent conflicts with protected content"
            );
            match self.blobs.symlink_metadata(digest.hex()) {
                Ok(_) => {
                    filesystem::owned_remove_blob(&self.blobs, digest.hex(), garbage.size)?;
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
