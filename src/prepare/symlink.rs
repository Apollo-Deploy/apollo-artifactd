use anyhow::{Result, bail, ensure};
use std::path::{Component, Path, PathBuf};

const MAX_TARGET: usize = 4096;

/// Validate a layer symlink and rewrite absolute targets into rootfs-relative
/// form. Relative targets retain their literal bytes: components such as
/// `a/../b` can have different meaning when `a` is itself a symlink.
pub(super) fn safe_target(path: &Path, bytes: &[u8]) -> Result<String> {
    ensure!(
        !bytes.is_empty() && bytes.len() <= MAX_TARGET && !bytes.contains(&0),
        "invalid link target"
    );
    let target = std::str::from_utf8(bytes)?;
    let parent = path.parent().unwrap_or_else(|| Path::new(""));
    let absolute = Path::new(target).is_absolute();
    if !absolute {
        validate_relative(parent, Path::new(target))?;
        return Ok(target.to_owned());
    }

    validate_absolute(Path::new(target))?;
    // Preserve intermediate dot components for the same reason relative
    // targets are preserved. A simple absolute path can be shortened safely.
    let raw_suffix = target.trim_start_matches('/');
    let has_dot_segment = raw_suffix
        .split('/')
        .any(|component| component == "." || component == "..")
        || raw_suffix.ends_with('/');
    if has_dot_segment {
        let prefix = "../".repeat(components(parent)?.len());
        let result = format!("{prefix}{raw_suffix}");
        ensure!(result.len() <= MAX_TARGET, "link target too long");
        return Ok(result);
    }
    let joined = PathBuf::from(target);
    let canonical = normalize(&joined, absolute)?;
    let parent_parts = components(parent)?;
    let target_parts = components(&canonical)?;
    let common = parent_parts
        .iter()
        .zip(&target_parts)
        .take_while(|(left, right)| left == right)
        .count();
    let mut result = PathBuf::new();
    for _ in common..parent_parts.len() {
        result.push("..");
    }
    for component in &target_parts[common..] {
        result.push(component);
    }
    let result = if result.as_os_str().is_empty() {
        ".".to_owned()
    } else {
        result.to_string_lossy().into_owned()
    };
    ensure!(result.len() <= MAX_TARGET, "link target too long");
    Ok(result)
}

fn validate_relative(parent: &Path, target: &Path) -> Result<()> {
    let mut depth = components(parent)?.len();
    for component in target.components() {
        match component {
            Component::Normal(_) => depth += 1,
            Component::CurDir => {}
            Component::ParentDir => {
                ensure!(depth > 0, "symlink escape");
                depth -= 1;
            }
            _ => bail!("invalid symlink target"),
        }
    }
    Ok(())
}

fn validate_absolute(target: &Path) -> Result<()> {
    let mut depth = 0usize;
    for component in target.components() {
        match component {
            Component::RootDir | Component::CurDir => {}
            Component::Normal(_) => depth += 1,
            Component::ParentDir => {
                ensure!(depth > 0, "symlink escape");
                depth -= 1;
            }
            Component::Prefix(_) => bail!("invalid symlink target"),
        }
    }
    ensure!(depth > 0, "invalid symlink target");
    Ok(())
}

fn normalize(path: &Path, absolute: bool) -> Result<PathBuf> {
    let mut output = PathBuf::new();
    for component in path.components() {
        match component {
            Component::RootDir if absolute => {}
            Component::Normal(part) => output.push(part),
            Component::CurDir => {}
            Component::ParentDir => {
                ensure!(output.pop(), "symlink escape");
            }
            Component::RootDir | Component::Prefix(_) => bail!("invalid symlink target"),
        }
    }
    ensure!(!output.as_os_str().is_empty(), "invalid symlink target");
    Ok(output)
}

fn components(path: &Path) -> Result<Vec<String>> {
    path.components()
        .map(|component| match component {
            Component::Normal(value) => value
                .to_str()
                .map(str::to_owned)
                .ok_or_else(|| anyhow::anyhow!("non-UTF8 symlink target")),
            _ => bail!("invalid normalized symlink path"),
        })
        .collect()
}
