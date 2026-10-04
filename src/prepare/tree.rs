//! Inspect, sync, freeze and remove only owned preparation trees.
use crate::filesystem;
use anyhow::{Result, ensure};
use cap_std::fs::{Dir, MetadataExt};
use rustix::fs::{Mode, OFlags};
use sha2::{Digest, Sha256};

pub(super) fn names(dir: &Dir) -> Result<Vec<String>> {
    let mut names = Vec::new();
    for entry in dir.entries()? {
        ensure!(names.len() < 100_000, "prepared directory budget exceeded");
        names.push(
            entry?
                .file_name()
                .into_string()
                .map_err(|_| anyhow::anyhow!("non-UTF8 prepared name"))?,
        );
    }
    names.sort();
    Ok(names)
}
fn owned(dir: &Dir, name: &str) -> Result<cap_std::fs::Metadata> {
    let meta = dir.symlink_metadata(name)?;
    ensure!(
        meta.uid() == rustix::process::geteuid().as_raw()
            && (meta.is_symlink() || meta.mode() & 0o077 == 0),
        "foreign prepared entry"
    );
    ensure!(
        meta.is_dir() || meta.is_file() || meta.is_symlink(),
        "special prepared entry"
    );
    if meta.is_file() {
        ensure!(meta.nlink() == 1, "foreign hardlink");
    }
    Ok(meta)
}
fn child(dir: &Dir, name: &str) -> Result<Dir> {
    let fd = rustix::fs::openat(
        dir,
        name,
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )?;
    Ok(Dir::from_std_file(std::fs::File::from(fd)))
}
pub(super) fn remove(dir: &Dir, name: &str) -> Result<()> {
    remove_inner(dir, name, 0)
}
fn remove_inner(dir: &Dir, name: &str, depth: usize) -> Result<()> {
    ensure!(depth <= 128, "prepared depth exceeded");
    let meta = owned(dir, name)?;
    if meta.is_dir() {
        let sub = child(dir, name)?;
        rustix::fs::fchmod(&sub, Mode::from_raw_mode(0o700))?;
        for n in names(&sub)? {
            remove_inner(&sub, &n, depth + 1)?;
        }
        dir.remove_dir(name)?;
    } else {
        dir.remove_file(name)?;
    }
    filesystem::sync(dir)
}

pub(super) fn freeze(root: &Dir) -> Result<String> {
    links(root, root, std::path::Path::new(""), 0)?;
    let mut hash = Sha256::new();
    visit(root, &mut hash, true, 0, &mut 0)?;
    Ok(hex::encode(hash.finalize()))
}
pub(super) fn verify(root: &Dir) -> Result<String> {
    let mut hash = Sha256::new();
    visit(root, &mut hash, false, 0, &mut 0)?;
    Ok(hex::encode(hash.finalize()))
}
fn visit(
    dir: &Dir,
    hash: &mut Sha256,
    freeze: bool,
    depth: usize,
    count: &mut usize,
) -> Result<()> {
    ensure!(depth <= 128, "prepared depth exceeded");
    for name in names(dir)? {
        *count += 1;
        ensure!(*count <= 100_000, "prepared tree budget exceeded");
        let meta = owned(dir, &name)?;
        hash.update((name.len() as u64).to_le_bytes());
        hash.update(name.as_bytes());
        if meta.is_dir() {
            hash.update(b"D");
            let sub = child(dir, &name)?;
            visit(&sub, hash, freeze, depth + 1, count)?;
            hash.update(b"E");
        } else if meta.is_symlink() {
            hash.update(b"L");
            let target = dir.read_link(&name)?;
            let text = target
                .to_str()
                .ok_or_else(|| anyhow::anyhow!("non-UTF8 link"))?;
            hash.update((text.len() as u64).to_le_bytes());
            hash.update(text.as_bytes());
        } else {
            hash.update(b"F");
            hash.update((meta.mode() & 0o111).to_le_bytes());
            hash.update(meta.len().to_le_bytes());
            let mut file = filesystem::read(dir, &name)?;
            let (digest, size) = crate::cas::hash(&mut file, 4 << 30)?;
            ensure!(size == meta.len(), "prepared size changed");
            hash.update(digest.as_str().as_bytes());
        }
    }
    if freeze {
        rustix::fs::fchmod(dir, Mode::from_raw_mode(0o500))?;
    }
    let stat = rustix::fs::fstat(dir)?;
    ensure!(stat.st_mode & 0o222 == 0, "writable prepared directory");
    filesystem::sync(dir)
}

fn links(root: &Dir, dir: &Dir, base: &std::path::Path, depth: usize) -> Result<()> {
    ensure!(depth <= 128, "link graph depth exceeded");
    for name in names(dir)? {
        let meta = owned(dir, &name)?;
        let path = base.join(&name);
        if meta.is_dir() {
            links(root, &child(dir, &name)?, &path, depth + 1)?;
        } else if meta.is_symlink() {
            resolve_link(root, &path)?;
        }
    }
    Ok(())
}
fn resolve_link(root: &Dir, path: &std::path::Path) -> Result<()> {
    use std::collections::VecDeque;
    let mut pending: VecDeque<String> = path
        .components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect();
    let mut resolved = Vec::<String>::new();
    let mut followed = 0;
    while let Some(component) = pending.pop_front() {
        if component == "." {
            continue;
        }
        if component == ".." {
            ensure!(resolved.pop().is_some(), "transitive symlink escape");
            continue;
        }
        ensure!(
            !component.is_empty() && component != "/",
            "absolute link escape"
        );
        let mut parent = root.try_clone()?;
        let mut missing = false;
        for name in &resolved {
            match child(&parent, name) {
                Ok(dir) => parent = dir,
                Err(e)
                    if e.downcast_ref::<rustix::io::Errno>() == Some(&rustix::io::Errno::NOENT) =>
                {
                    missing = true;
                    break;
                }
                Err(e) => return Err(e),
            }
        }
        let meta = if missing {
            None
        } else {
            match parent.symlink_metadata(&component) {
                Ok(meta) => Some(meta),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
                Err(e) => return Err(e.into()),
            }
        };
        if meta.is_some_and(|m| m.is_symlink()) {
            followed += 1;
            ensure!(followed <= 40, "symlink cycle/limit");
            let target = parent.read_link(&component)?;
            ensure!(!target.is_absolute(), "absolute symlink escape");
            let parts: Vec<_> = target
                .components()
                .map(|c| c.as_os_str().to_string_lossy().into_owned())
                .collect();
            for part in parts.into_iter().rev() {
                pending.push_front(part);
            }
        } else {
            resolved.push(component);
        }
        ensure!(
            resolved.len() + pending.len() <= 256,
            "link resolution budget exceeded"
        );
    }
    Ok(())
}
