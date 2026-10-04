use super::{GC_REFERENCE_EPOCH, MAX_RECORD, METADATA, TABLES};
use anyhow::{Context, Result, ensure};
use serde::{Serialize, de::DeserializeOwned};

pub(super) fn validate_table(name: &'static str) -> Result<()> {
    ensure!(TABLES.contains(&name), "unknown state table: {name}");
    Ok(())
}

pub(super) fn validate_key(key: &str) -> Result<()> {
    ensure!(!key.is_empty() && key.len() <= 4096, "invalid state key");
    ensure!(!key.as_bytes().contains(&0), "state key contains NUL");
    Ok(())
}

pub(super) fn validate_mutation(name: &'static str, key: &str) -> Result<()> {
    validate_table(name)?;
    validate_key(key)?;
    ensure!(
        !(name == METADATA && matches!(key, "schema_version" | GC_REFERENCE_EPOCH)),
        "schema version is immutable"
    );
    Ok(())
}

pub(super) fn encode<T: Serialize>(value: &T) -> Result<Vec<u8>> {
    let bytes = serde_json::to_vec(value).context("encode state record")?;
    ensure!(bytes.len() <= MAX_RECORD, "state record exceeds 64 KiB");
    Ok(bytes)
}

pub(super) fn decode<T: DeserializeOwned>(bytes: &[u8]) -> Result<T> {
    ensure!(bytes.len() <= MAX_RECORD, "state record exceeds 64 KiB");
    serde_json::from_slice(bytes).context("decode state record")
}
