//! Preparation never executes image configuration or customer commands.
mod layers;
mod recovery;
mod tree;
use crate::{Store, filesystem};
use anyhow::{Result, ensure};
use artifactd_protocol::{ArtifactDigest, Platform, PreparedArtifactId};
use rusqlite::OptionalExtension;
use sha2::{Digest, Sha256};

const FORMAT: &str = "artifactd-rootfs-v1";
impl Store {
    pub fn prepare(
        &mut self,
        source: &ArtifactDigest,
        platform: &Platform,
    ) -> Result<PreparedArtifactId> {
        let image = self.resolve(source, platform)?;
        ensure!(
            image.digest == *source,
            "preparation requires selected manifest identity"
        );
        self.admit_oci(source, platform)?;
        let p = serde_json::to_string(platform)?;
        let identity = format!("{FORMAT}\n{source}\n{p}\n");
        let id = hex::encode(Sha256::digest(identity.as_bytes()));
        let phase: Option<String> = self
            .db
            .query_row("SELECT phase FROM prepared WHERE id=?1", [&id], |r| {
                r.get(0)
            })
            .optional()?;
        if phase.as_deref() == Some("complete") {
            let dir = self.prepared.open_dir(&id)?;
            let expected: String =
                self.db
                    .query_row("SELECT tree_digest FROM prepared WHERE id=?1", [&id], |r| {
                        r.get(0)
                    })?;
            ensure!(
                tree::verify(&dir)? == expected,
                "prepared integrity mismatch"
            );
            return PreparedArtifactId::try_from(id).map_err(anyhow::Error::msg);
        }
        if phase.is_some() {
            self.recover_prepared(4096)?;
            return self.prepare(source, platform);
        }
        let staging_name = format!("{}.staging", uuid::Uuid::new_v4());
        self.db.execute("INSERT INTO prepared(id,manifest,platform,phase,staging) VALUES (?1,?2,?3,'intent',?4)", rusqlite::params![id,source.as_str(),p,staging_name])?;
        rustix::fs::mkdirat(
            &self.prepared,
            &staging_name,
            rustix::fs::Mode::from_raw_mode(0o700),
        )?;
        filesystem::sync(&self.prepared)?;
        let staging = filesystem::directory(&self.prepared, &staging_name)?;
        let size = self.extract_layers(&image, &staging)?;
        let tree_digest = tree::freeze(&staging)?;
        self.db.execute(
            "UPDATE prepared SET tree_digest=?2,size=?3 WHERE id=?1",
            rusqlite::params![id, tree_digest, i64::try_from(size)?],
        )?;
        filesystem::sync(&staging)?;
        self.prepared.rename(&staging_name, &self.prepared, &id)?;
        filesystem::sync(&self.prepared)?;
        self.db
            .execute("UPDATE prepared SET phase='complete' WHERE id=?1", [&id])?;
        PreparedArtifactId::try_from(id).map_err(anyhow::Error::msg)
    }

    pub fn open_prepared(&self, id: &PreparedArtifactId, lease: &str) -> Result<std::fs::File> {
        let manifest: String = self.db.query_row(
            "SELECT manifest FROM prepared WHERE id=?1 AND phase='complete'",
            [id.as_str()],
            |r| r.get(0),
        )?;
        ensure!(
            self.leased(lease, &manifest.parse()?)?,
            "prepared artifact requires a covering lease"
        );
        let dir = self.prepared.open_dir(id.as_str())?;
        let expected: String = self.db.query_row(
            "SELECT tree_digest FROM prepared WHERE id=?1",
            [id.as_str()],
            |r| r.get(0),
        )?;
        ensure!(
            tree::verify(&dir)? == expected,
            "prepared integrity mismatch"
        );
        let fd = rustix::fs::openat(
            &self.prepared,
            id.as_str(),
            rustix::fs::OFlags::RDONLY
                | rustix::fs::OFlags::DIRECTORY
                | rustix::fs::OFlags::NOFOLLOW
                | rustix::fs::OFlags::CLOEXEC,
            rustix::fs::Mode::empty(),
        )?;
        let stat = rustix::fs::fstat(&fd)?;
        ensure!(
            stat.st_uid == rustix::process::geteuid().as_raw() && stat.st_mode & 0o222 == 0,
            "prepared output is writable or foreign"
        );
        Ok(std::fs::File::from(fd))
    }
}
