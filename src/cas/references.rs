use super::Store;
use anyhow::{Result, ensure};
use artifactd_protocol::ArtifactDigest;
use rusqlite::OptionalExtension;

const LIVE: &str = "WITH RECURSIVE live(digest) AS (SELECT digest FROM pins UNION SELECT digest FROM leases UNION SELECT manifest FROM prepared UNION SELECT edges.child FROM edges JOIN live ON edges.parent=live.digest)";

impl Store {
    pub fn pin(&mut self, id: &str, digest: &ArtifactDigest) -> Result<()> {
        self.reference("pins", id, digest)
    }
    pub fn lease(&mut self, id: &str, digest: &ArtifactDigest) -> Result<()> {
        self.reference("leases", id, digest)
    }

    fn reference(&mut self, table: &str, id: &str, digest: &ArtifactDigest) -> Result<()> {
        ensure!(
            id.len() <= 128 && !id.is_empty(),
            "invalid reference identity"
        );
        let count: u32 = self
            .db
            .query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))?;
        ensure!(count < 100_000, "reference count limit");
        self.open_blob(digest)?;
        self.verify_graph_if_known(digest)?;
        let existing: Option<String> = self
            .db
            .query_row(
                &format!("SELECT digest FROM {table} WHERE id=?1"),
                [id],
                |r| r.get(0),
            )
            .optional()?;
        ensure!(
            existing.as_ref().is_none_or(|v| v == digest.as_str()),
            "reference identity conflict"
        );
        self.db.execute(
            &format!("INSERT INTO {table}(id,digest) VALUES (?1,?2) ON CONFLICT(id) DO NOTHING"),
            rusqlite::params![id, digest.as_str()],
        )?;
        Ok(())
    }

    pub fn unpin(&mut self, id: &str) -> Result<()> {
        self.db.execute("DELETE FROM pins WHERE id=?1", [id])?;
        Ok(())
    }
    pub fn release(&mut self, id: &str) -> Result<()> {
        self.db.execute("DELETE FROM leases WHERE id=?1", [id])?;
        Ok(())
    }

    pub fn leased(&self, id: &str, digest: &ArtifactDigest) -> Result<bool> {
        let sql = "WITH RECURSIVE live(digest) AS (SELECT digest FROM leases WHERE id=?1 UNION SELECT edges.child FROM edges JOIN live ON edges.parent=live.digest) SELECT EXISTS(SELECT 1 FROM live WHERE digest=?2)";
        Ok(self
            .db
            .query_row(sql, rusqlite::params![id, digest.as_str()], |r| r.get(0))?)
    }

    pub(crate) fn protected(&self, digest: &ArtifactDigest) -> Result<bool> {
        Ok(self.db.query_row(
            &format!("{LIVE} SELECT EXISTS(SELECT 1 FROM live WHERE digest=?1)"),
            [digest.as_str()],
            |r| r.get(0),
        )?)
    }

    pub(crate) fn verify_live_graphs(&self) -> Result<()> {
        // Refuse deletion if the durable reference graph disagrees with OCI bytes.
        // This is bounded by the reference quota, but still needs incremental work
        // scheduling before a production release.
        let mut q = self.db.prepare("SELECT digest FROM pins UNION SELECT digest FROM leases UNION SELECT manifest FROM prepared")?;
        for digest in q.query_map([], |r| r.get::<_, String>(0))? {
            let digest = digest?.parse()?;
            self.open_blob(&digest)?;
            self.verify_graph_if_known(&digest)?;
        }
        Ok(())
    }

    /// Serialized with all references/imports. Only SQL-recorded verified inodes
    /// can be collected; directory enumeration never authorizes deletion.
    pub fn gc(&mut self, max: u32) -> Result<u32> {
        ensure!(max > 0 && max <= 4096, "invalid gc bound");
        self.verify_live_graphs()?;
        self.recover_gc(max)?;
        self.gc_prepared(max)?;
        let candidates = {
            let mut q = self.db.prepare(&format!("{LIVE} SELECT digest,size FROM blobs WHERE digest NOT IN (SELECT digest FROM live) AND digest NOT IN (SELECT digest FROM imports) LIMIT ?1"))?;
            q.query_map([max], |r| {
                Ok((r.get::<_, String>(0)?, crate::state::unsigned(r, 1)?))
            })?
            .collect::<Result<Vec<_>, _>>()?
        };
        let tx = self.db.transaction()?;
        for (digest, size) in &candidates {
            tx.execute(
                "INSERT OR IGNORE INTO gc(digest,size) VALUES (?1,?2)",
                rusqlite::params![digest, i64::try_from(*size)?],
            )?;
        }
        tx.commit()?;
        self.recover_gc(max)?;
        Ok(candidates.len() as u32)
    }
}
