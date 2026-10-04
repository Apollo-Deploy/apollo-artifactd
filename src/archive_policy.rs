//! Policy for metadata interpreted by the mature tar parser.
use anyhow::{Result, bail, ensure};
use std::{
    collections::BTreeSet,
    io::Read,
    path::{Component, Path},
};

pub(crate) fn validate_pax<R: Read>(entry: &mut tar::Entry<'_, R>) -> Result<()> {
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

pub(crate) fn safe_path(bytes: &[u8]) -> Result<std::path::PathBuf> {
    ensure!(
        bytes.len() <= 4096 && !bytes.contains(&0),
        "invalid archive path"
    );
    let text = std::str::from_utf8(bytes)?;
    let path = Path::new(text);
    ensure!(
        !text.is_empty() && !path.is_absolute() && path.components().count() <= 128,
        "invalid archive path"
    );
    let mut normalized = std::path::PathBuf::new();
    for component in path.components() {
        match component {
            Component::Normal(name) => normalized.push(name),
            Component::CurDir => {}
            _ => bail!("absolute/traversing archive path"),
        }
    }
    Ok(normalized)
}
