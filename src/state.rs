//! SQLite owns durable intents and references; no custom database format.
use crate::filesystem;
use anyhow::{Result, ensure};
use cap_std::fs::Dir;
use rusqlite::Connection;
use std::{os::fd::AsRawFd, path::Path};

pub fn open(root: &Dir, original: &Path) -> Result<Connection> {
    let fd = rustix::fs::openat(
        root,
        "state.sqlite",
        rustix::fs::OFlags::RDWR
            | rustix::fs::OFlags::CREATE
            | rustix::fs::OFlags::NOFOLLOW
            | rustix::fs::OFlags::CLOEXEC,
        rustix::fs::Mode::from_raw_mode(0o600),
    )?;
    filesystem::private_fd(&fd, false)?;
    #[cfg(target_os = "linux")]
    let path = {
        let _ = original;
        format!("/proc/self/fd/{}/state.sqlite", root.as_raw_fd())
    };
    #[cfg(not(target_os = "linux"))]
    let path = {
        let _ = root.as_raw_fd();
        original.join("state.sqlite").to_string_lossy().into_owned()
    };
    for name in [
        "state.sqlite-wal",
        "state.sqlite-shm",
        "state.sqlite-journal",
    ] {
        match rustix::fs::openat(
            root,
            name,
            rustix::fs::OFlags::RDONLY | rustix::fs::OFlags::NOFOLLOW | rustix::fs::OFlags::CLOEXEC,
            rustix::fs::Mode::empty(),
        ) {
            Ok(fd) => filesystem::private_fd(&fd, false)?,
            Err(rustix::io::Errno::NOENT) => {}
            Err(e) => return Err(e.into()),
        }
    }
    let db = Connection::open(path)?;
    db.pragma_update(None, "journal_mode", "WAL")?;
    db.pragma_update(None, "synchronous", "FULL")?;
    db.pragma_update(None, "foreign_keys", "ON")?;
    db.busy_timeout(std::time::Duration::from_secs(5))?;
    let version: u32 = db.pragma_query_value(None, "user_version", |r| r.get(0))?;
    ensure!(version <= 1, "unsupported future state schema");
    db.execute_batch("BEGIN IMMEDIATE;
        CREATE TABLE IF NOT EXISTS blobs(digest TEXT PRIMARY KEY, size INTEGER NOT NULL CHECK(size >= 0));
        CREATE TABLE IF NOT EXISTS imports(id TEXT PRIMARY KEY, temp TEXT UNIQUE NOT NULL, digest TEXT NOT NULL, size INTEGER NOT NULL);
        CREATE TABLE IF NOT EXISTS roots(digest TEXT PRIMARY KEY REFERENCES blobs(digest), kind TEXT NOT NULL, platform TEXT);
        CREATE TABLE IF NOT EXISTS edges(parent TEXT NOT NULL REFERENCES blobs(digest), child TEXT NOT NULL REFERENCES blobs(digest), PRIMARY KEY(parent, child));
        CREATE TABLE IF NOT EXISTS pins(id TEXT PRIMARY KEY, digest TEXT NOT NULL REFERENCES blobs(digest));
        CREATE TABLE IF NOT EXISTS leases(id TEXT PRIMARY KEY, digest TEXT NOT NULL REFERENCES blobs(digest));
        CREATE TABLE IF NOT EXISTS prepared(id TEXT PRIMARY KEY, manifest TEXT NOT NULL REFERENCES roots(digest), platform TEXT NOT NULL, phase TEXT NOT NULL, tree_digest TEXT, staging TEXT NOT NULL, size INTEGER NOT NULL DEFAULT 0);
        CREATE TABLE IF NOT EXISTS gc(digest TEXT PRIMARY KEY, size INTEGER NOT NULL);
        CREATE TABLE IF NOT EXISTS operations(seq INTEGER PRIMARY KEY AUTOINCREMENT, id TEXT UNIQUE NOT NULL, request TEXT NOT NULL, phase TEXT NOT NULL, result TEXT);
        PRAGMA user_version=1;
        COMMIT;")?;
    filesystem::sync(root)?;
    Ok(db)
}

pub(crate) fn unsigned(row: &rusqlite::Row<'_>, index: usize) -> rusqlite::Result<u64> {
    let value: i64 = row.get(index)?;
    u64::try_from(value).map_err(|_| rusqlite::Error::IntegralValueOutOfRange(index, value))
}
