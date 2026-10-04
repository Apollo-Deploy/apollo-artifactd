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

mod records;
pub use records::{Blob, Garbage, Imported, Operation, Prepared, Reference, Root};

pub const CURRENT_SCHEMA: u32 = 2;
pub const MAX_RECORD: usize = 64 * 1024;
const MAX_SCAN: usize = 4096;
const METADATA: &str = "metadata";
const TABLES: [&str; 11] = [
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
    METADATA,
];

type Table = TableDefinition<'static, &'static str, &'static [u8]>;

fn table(name: &'static str) -> Table {
    TableDefinition::new(name)
}

fn validate_table(name: &'static str) -> Result<()> {
    ensure!(TABLES.contains(&name), "unknown state table: {name}");
    Ok(())
}

fn validate_key(key: &str) -> Result<()> {
    ensure!(!key.is_empty() && key.len() <= 4096, "invalid state key");
    ensure!(!key.as_bytes().contains(&0), "state key contains NUL");
    Ok(())
}

fn validate_mutation(name: &'static str, key: &str) -> Result<()> {
    validate_table(name)?;
    validate_key(key)?;
    ensure!(
        !(name == METADATA && key == "schema_version"),
        "schema version is immutable"
    );
    Ok(())
}

fn encode<T: Serialize>(value: &T) -> Result<Vec<u8>> {
    let bytes = serde_json::to_vec(value).context("encode state record")?;
    ensure!(bytes.len() <= MAX_RECORD, "state record exceeds 64 KiB");
    Ok(bytes)
}

fn decode<T: DeserializeOwned>(bytes: &[u8]) -> Result<T> {
    ensure!(bytes.len() <= MAX_RECORD, "state record exceeds 64 KiB");
    serde_json::from_slice(bytes).context("decode state record")
}

/// A single owning redb database. The parent capability is retained so every
/// committed mutation can sync the directory entry without reopening a path.
pub struct State {
    db: Database,
    root: Dir,
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
        let state = Self {
            db,
            root: root.try_clone()?,
        };
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
            Some(version) => {
                ensure!(
                    version == CURRENT_SCHEMA,
                    "unsupported state schema {version}"
                );
            }
            None => {
                let mut metadata = write.open_table(table(METADATA))?;
                metadata.insert("schema_version", encode(&CURRENT_SCHEMA)?.as_slice())?;
            }
        }
        write.set_durability(Durability::Immediate)?;
        write.commit()?;
        filesystem::sync(&self.root)
    }

    pub fn get<T: DeserializeOwned>(&self, name: &'static str, key: &str) -> Result<Option<T>> {
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
        let write = self.db.begin_write()?;
        let mut tx = StateTx { write };
        let result = operation(&mut tx)?;
        tx.write.set_durability(Durability::Immediate)?;
        tx.write.commit()?;
        filesystem::sync(&self.root)?;
        Ok(result)
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
        let mut table = self.write.open_table(table(name))?;
        table.insert(key, bytes.as_slice())?;
        Ok(())
    }

    pub fn remove(&mut self, name: &'static str, key: &str) -> Result<()> {
        validate_mutation(name, key)?;
        let mut table = self.write.open_table(table(name))?;
        table.remove(key)?;
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
