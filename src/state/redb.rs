//! Capability-opened redb state for the standalone artifact service.
//!
//! This adapter deliberately keeps the database mechanism in redb. Apollo policy is
//! limited to the table allow-list, schema version, bounded JSON values, and the
//! transaction boundary used by callers.
use crate::filesystem;
use anyhow::{Context, Result, ensure};
use cap_std::fs::Dir;
use redb::{
    Builder, Database, Durability, ReadableDatabase, ReadableTable, ReadableTableMetadata,
    TableDefinition, WriteTransaction,
};
use rustix::fs::{Mode, OFlags};
use serde::{Serialize, de::DeserializeOwned};
use std::{fs::File, ops::Bound};
mod codec;
mod integrity;
mod records;
use codec::{decode, encode, validate_key, validate_mutation, validate_table};
pub use records::{
    Blob, Garbage, Imported, LeaseState, Operation, PeerIdentity, Prepared, Reference, Root,
};
pub const CURRENT_SCHEMA: u32 = 3;
pub const MAX_RECORD: usize = 64 * 1024;
const MAX_SCAN: usize = 4096;
const METADATA: &str = "metadata";
const GC_REFERENCE_EPOCH: &str = "gc_reference_epoch";
const TABLES: [&str; 12] = [
    "blobs",
    "imports",
    "roots",
    "edges",
    "pins",
    "leases",
    "prepared",
    "gc",
    "operations",
    "registry",
    "gc_marks",
    METADATA,
];

type Table = TableDefinition<'static, &'static str, &'static [u8]>;

fn table(name: &'static str) -> Table {
    TableDefinition::new(name)
}
/// A single owning redb database. The parent capability is retained so every
/// committed mutation can sync the directory entry without reopening a path.
pub struct State {
    db: Database,
    root: Dir,
    healthy: bool,
}

impl State {
    /// Opens `state.redb` relative to a verified private store capability.
    pub fn open(root: &Dir) -> Result<Self> {
        let fd = rustix::fs::openat(
            root,
            "state.redb",
            OFlags::RDWR | OFlags::CREATE | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::from_raw_mode(0o600),
        )?;
        filesystem::private_fd(&fd, false)?;
        let db = Builder::new()
            .set_cache_size(8 * 1024 * 1024)
            .create_file(File::from(fd))
            .context("open redb state")?;
        let mut state = Self {
            db,
            root: root.try_clone()?,
            healthy: true,
        };
        ensure!(
            state.check_integrity()?,
            "database repaired during open; reconciliation requires operator review"
        );
        state.initialize_schema()?;
        Ok(state)
    }

    fn initialize_schema(&self) -> Result<()> {
        let write = self.db.begin_write()?;
        let mut write = write;
        for name in TABLES {
            let _ = write.open_table(table(name))?;
        }
        let existing: Option<u32> = {
            let metadata = write.open_table(table(METADATA))?;
            metadata
                .get("schema_version")?
                .map(|value| decode(value.value()))
                .transpose()?
        };
        match existing {
            Some(2) => {
                ensure!(
                    write.open_table(table("gc_marks"))?.len()? == 0,
                    "schema 2 gc marks must be empty"
                );
                let mut metadata = write.open_table(table(METADATA))?;
                metadata.insert(GC_REFERENCE_EPOCH, encode(&0u64)?.as_slice())?;
                metadata.insert("schema_version", encode(&CURRENT_SCHEMA)?.as_slice())?;
            }
            Some(3) => {
                let metadata = write.open_table(table(METADATA))?;
                let epoch = metadata
                    .get(GC_REFERENCE_EPOCH)?
                    .map(|value| decode::<u64>(value.value()))
                    .transpose()?;
                ensure!(epoch.is_some(), "missing GC reference epoch");
            }
            Some(version) => ensure!(
                version == CURRENT_SCHEMA,
                "unsupported state schema {version}"
            ),
            None => {
                let mut metadata = write.open_table(table(METADATA))?;
                metadata.insert("schema_version", encode(&CURRENT_SCHEMA)?.as_slice())?;
                metadata.insert(GC_REFERENCE_EPOCH, encode(&0u64)?.as_slice())?;
            }
        }
        write.set_two_phase_commit(true);
        write.set_durability(Durability::Immediate)?;
        write.commit()?;
        filesystem::sync(&self.root)
    }

    pub fn get<T: DeserializeOwned>(&self, name: &'static str, key: &str) -> Result<Option<T>> {
        self.require_healthy()?;
        validate_table(name)?;
        validate_key(key)?;
        let read = self.db.begin_read()?;
        let table = read.open_table(table(name))?;
        table
            .get(key)?
            .map(|value| decode(value.value()))
            .transpose()
    }

    pub fn count(&self, name: &'static str) -> Result<u64> {
        self.require_healthy()?;
        validate_table(name)?;
        let read = self.db.begin_read()?;
        Ok(read.open_table(table(name))?.len()?)
    }

    pub fn scan<T: DeserializeOwned>(
        &self,
        name: &'static str,
        after: Option<&str>,
        limit: usize,
    ) -> Result<Vec<(String, T)>> {
        self.require_healthy()?;
        let read = self.db.begin_read()?;
        scan_table(&read.open_table(table_checked(name)?)?, after, limit)
    }

    pub fn put<T: Serialize>(&self, name: &'static str, key: &str, value: &T) -> Result<()> {
        self.transaction(|tx| tx.put(name, key, value))
    }

    pub fn remove(&self, name: &'static str, key: &str) -> Result<()> {
        self.transaction(|tx| tx.remove(name, key))
    }

    pub fn transaction<R>(&self, operation: impl FnOnce(&mut StateTx) -> Result<R>) -> Result<R> {
        self.require_healthy()?;
        let write = self.db.begin_write()?;
        let mut tx = StateTx {
            write,
            dirty: false,
        };
        let result = operation(&mut tx)?;
        tx.commit_epoch_if_dirty()?;
        tx.write.set_two_phase_commit(true);
        tx.write.set_durability(Durability::Immediate)?;
        tx.write.commit()?;
        filesystem::sync(&self.root)?;
        Ok(result)
    }

    pub fn is_healthy(&self) -> bool {
        self.healthy
    }

    fn require_healthy(&self) -> Result<()> {
        ensure!(
            self.healthy,
            "state database is quarantined; restart required"
        );
        Ok(())
    }
}

fn table_checked(name: &'static str) -> Result<Table> {
    validate_table(name)?;
    Ok(table(name))
}
fn scan_table<T: DeserializeOwned>(
    table: &redb::ReadOnlyTable<&'static str, &'static [u8]>,
    after: Option<&str>,
    limit: usize,
) -> Result<Vec<(String, T)>> {
    ensure!(
        (1..=MAX_SCAN).contains(&limit),
        "scan limit must be 1..=4096"
    );
    let mut output = Vec::with_capacity(limit);
    let range = match after {
        Some(key) => {
            let bounds: (Bound<&str>, Bound<&str>) = (Bound::Excluded(key), Bound::Unbounded);
            table.range::<&str>(bounds)?
        }
        None => {
            let bounds: (Bound<&str>, Bound<&str>) = (Bound::Unbounded, Bound::Unbounded);
            table.range::<&str>(bounds)?
        }
    };
    for item in range.take(limit) {
        let (key, value) = item?;
        output.push((key.value().to_owned(), decode(value.value())?));
    }
    Ok(output)
}

pub struct StateTx {
    write: WriteTransaction,
    dirty: bool,
}

impl StateTx {
    pub fn get<T: DeserializeOwned>(&mut self, name: &'static str, key: &str) -> Result<Option<T>> {
        validate_table(name)?;
        validate_key(key)?;
        let table = self.write.open_table(table(name))?;
        table
            .get(key)?
            .map(|value| decode(value.value()))
            .transpose()
    }

    pub fn put<T: Serialize>(&mut self, name: &'static str, key: &str, value: &T) -> Result<()> {
        validate_mutation(name, key)?;
        let bytes = encode(value)?;
        let changed = {
            let table = self.write.open_table(table(name))?;
            table
                .get(key)?
                .is_none_or(|current| current.value() != bytes.as_slice())
        };
        if !changed {
            return Ok(());
        }
        if name == "prepared" {
            let previous = self.get::<Prepared>(name, key)?;
            let prepared: Prepared = decode(&bytes)?;
            self.dirty |= prepared_dirty(previous.as_ref(), &prepared);
        } else if matches!(name, "pins" | "leases" | "roots" | "edges") {
            self.dirty = true;
        }
        let mut table = self.write.open_table(table(name))?;
        table.insert(key, bytes.as_slice())?;
        Ok(())
    }

    pub fn remove(&mut self, name: &'static str, key: &str) -> Result<()> {
        validate_mutation(name, key)?;
        let existed = {
            let table = self.write.open_table(table(name))?;
            table.get(key)?.is_some()
        };
        if !existed {
            return Ok(());
        }
        if matches!(name, "pins" | "leases") {
            self.dirty = true;
        }
        let mut table = self.write.open_table(table(name))?;
        table.remove(key)?;
        Ok(())
    }

    fn commit_epoch_if_dirty(&mut self) -> Result<()> {
        if !self.dirty {
            return Ok(());
        }
        let current = self
            .get::<u64>(METADATA, GC_REFERENCE_EPOCH)?
            .ok_or_else(|| anyhow::anyhow!("missing GC reference epoch"))?;
        let next = current
            .checked_add(1)
            .ok_or_else(|| anyhow::anyhow!("GC reference epoch exhausted"))?;
        let bytes = encode(&next)?;
        let mut metadata = self.write.open_table(table(METADATA))?;
        metadata.insert(GC_REFERENCE_EPOCH, bytes.as_slice())?;
        Ok(())
    }

    pub fn count(&mut self, name: &'static str) -> Result<u64> {
        validate_table(name)?;
        Ok(self.write.open_table(table(name))?.len()?)
    }

    pub fn scan<T: DeserializeOwned>(
        &mut self,
        name: &'static str,
        after: Option<&str>,
        limit: usize,
    ) -> Result<Vec<(String, T)>> {
        validate_table(name)?;
        ensure!(
            (1..=MAX_SCAN).contains(&limit),
            "scan limit must be 1..=4096"
        );
        let table = self.write.open_table(table(name))?;
        let mut output = Vec::with_capacity(limit);
        let range = match after {
            Some(key) => {
                let bounds: (Bound<&str>, Bound<&str>) = (Bound::Excluded(key), Bound::Unbounded);
                table.range::<&str>(bounds)?
            }
            None => {
                let bounds: (Bound<&str>, Bound<&str>) = (Bound::Unbounded, Bound::Unbounded);
                table.range::<&str>(bounds)?
            }
        };
        for item in range.take(limit) {
            let (key, value) = item?;
            output.push((key.value().to_owned(), decode(value.value())?));
        }
        Ok(output)
    }
}

fn prepared_dirty(previous: Option<&Prepared>, next: &Prepared) -> bool {
    if next.phase == "complete" {
        return true;
    }
    if !matches!(next.phase.as_str(), "gc_intent" | "deleting") {
        return false;
    }
    previous.is_none_or(|old| {
        old.manifest != next.manifest
            || !matches!(old.phase.as_str(), "complete" | "gc_intent" | "deleting")
    })
}
