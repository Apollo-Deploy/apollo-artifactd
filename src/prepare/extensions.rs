//! Policy for metadata interpreted by the mature tar parser.
use anyhow::{Result, ensure};
use std::{collections::BTreeSet, io::Read};

pub(super) fn validate<R: Read>(entry: &mut tar::Entry<'_, R>) -> Result<()> {
    let Some(extensions) = entry.pax_extensions()? else {
        return Ok(());
    };
    let mut keys = BTreeSet::new();
    for extension in extensions {
        let extension = extension?;
        let key = extension.key()?;
        ensure!(!key.is_empty() && key.len() <= 128, "invalid PAX key");
        ensure!(keys.insert(key.to_owned()), "duplicate PAX key");
        if matches!(key, "size" | "uid" | "gid") {
            let value = extension.value_bytes();
            ensure!(
                !value.is_empty() && value.iter().all(u8::is_ascii_digit),
                "invalid numeric PAX value"
            );
            let _: u64 = std::str::from_utf8(value)?.parse()?;
        }
    }
    Ok(())
}
