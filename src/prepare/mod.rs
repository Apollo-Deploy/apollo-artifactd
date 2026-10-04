//! Preparation never executes image configuration or customer commands.
mod layers;
mod recovery;
mod stream;
mod tree;
use crate::{Store, filesystem};
use anyhow::{Result, ensure};
use artifactd_protocol::{ArtifactDigest, Platform, PreparedArtifactId};
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
        let existing = self.db.get::<crate::state::Prepared>("prepared", &id)?;
        let phase = existing.as_ref().map(|record| record.phase.clone());
        if phase.as_deref() == Some("complete") {
            let dir = self.prepared.open_dir(&id)?;
            let expected = existing
                .as_ref()
                .and_then(|record| record.tree_digest.clone())
                .ok_or_else(|| anyhow::anyhow!("missing prepared tree digest"))?;
            ensure!(
                tree::verify(&dir)? == expected,
                "prepared integrity mismatch"
            );
            return PreparedArtifactId::try_from(id).map_err(anyhow::Error::msg);
        }
        if phase.is_some() {
            self.recover_prepared_id(&id)?;
            if let Some(record) = self.db.get::<crate::state::Prepared>("prepared", &id)? {
                ensure!(
                    record.phase == "complete",
                    "prepared recovery left incomplete intent"
                );
                let expected = record
                    .tree_digest
                    .ok_or_else(|| anyhow::anyhow!("missing prepared tree digest"))?;
                let dir = self.prepared.open_dir(&id)?;
                ensure!(
                    tree::verify(&dir)? == expected,
                    "prepared integrity mismatch"
                );
                return PreparedArtifactId::try_from(id).map_err(anyhow::Error::msg);
            }
        }
        let staging_name = format!("{}.staging", uuid::Uuid::new_v4());
        self.db.put(
            "prepared",
            &id,
            &crate::state::Prepared {
                manifest: source.clone(),
                platform: platform.clone(),
                phase: "intent".to_owned(),
                tree_digest: None,
                staging: staging_name.clone(),
                size: 0,
            },
        )?;
        rustix::fs::mkdirat(
            &self.prepared,
            &staging_name,
            rustix::fs::Mode::from_raw_mode(0o700),
        )?;
        filesystem::sync(&self.prepared)?;
        let staging = filesystem::directory(&self.prepared, &staging_name)?;
        let size = self.extract_layers(&image, &staging)?;
        let tree_digest = tree::freeze(&staging)?;
        self.db.put(
            "prepared",
            &id,
            &crate::state::Prepared {
                manifest: source.clone(),
                platform: platform.clone(),
                phase: "intent".to_owned(),
                tree_digest: Some(tree_digest.clone()),
                staging: staging_name.clone(),
                size,
            },
        )?;
        filesystem::sync(&staging)?;
        self.prepared.rename(&staging_name, &self.prepared, &id)?;
        filesystem::sync(&self.prepared)?;
        let mut record = self
            .db
            .get::<crate::state::Prepared>("prepared", &id)?
            .ok_or_else(|| anyhow::anyhow!("prepared intent disappeared"))?;
        record.phase = "complete".to_owned();
        self.db.put("prepared", &id, &record)?;
        PreparedArtifactId::try_from(id).map_err(anyhow::Error::msg)
    }

    pub fn open_prepared(&self, id: &PreparedArtifactId, lease: &str) -> Result<std::fs::File> {
        let record = self
            .db
            .get::<crate::state::Prepared>("prepared", id.as_str())?
            .ok_or_else(|| anyhow::anyhow!("prepared artifact not found"))?;
        ensure!(
            record.phase == "complete",
            "prepared artifact is incomplete"
        );
        let manifest = record.manifest.to_string();
        ensure!(
            self.leased(lease, &manifest.parse()?)?,
            "prepared artifact requires a covering lease"
        );
        let dir = self.prepared.open_dir(id.as_str())?;
        let expected = record
            .tree_digest
            .ok_or_else(|| anyhow::anyhow!("missing prepared tree digest"))?;
        ensure!(
            tree::verify(&dir)? == expected.as_str(),
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
