use super::tree;
use crate::{Store, filesystem, oci::Image};
use anyhow::{Result, bail, ensure};
use cap_std::fs::Dir;
use sha2::{Digest, Sha256};
use std::{
    io::{Read, Write},
    path::{Component, Path},
};

struct VerifiedReader<R> {
    reader: R,
    hash: Sha256,
    size: u64,
    limit: u64,
}
impl<R: Read> Read for VerifiedReader<R> {
    fn read(&mut self, bytes: &mut [u8]) -> std::io::Result<usize> {
        let n = self.reader.read(bytes)?;
        self.size = self
            .size
            .checked_add(n as u64)
            .ok_or_else(|| std::io::Error::other("decompressed size overflow"))?;
        if self.size > self.limit {
            return Err(std::io::Error::other("decompression budget exceeded"));
        }
        self.hash.update(&bytes[..n]);
        Ok(n)
    }
}

impl Store {
    fn layer_reader(&self, layer: &oci_spec::image::Descriptor) -> Result<Box<dyn Read>> {
        let digest = self.descriptor(layer)?;
        let file = self.open_blob(&digest)?;
        Ok(match layer.media_type().to_string().as_str() {
            "application/vnd.oci.image.layer.v1.tar" => Box::new(file),
            "application/vnd.oci.image.layer.v1.tar+gzip" => {
                Box::new(flate2::read::MultiGzDecoder::new(file))
            }
            "application/vnd.oci.image.layer.v1.tar+zstd" => {
                Box::new(zstd::stream::read::Decoder::new(file)?)
            }
            _ => bail!("unsupported compression"),
        })
    }

    pub(crate) fn extract_layers(&self, image: &Image, root: &Dir) -> Result<u64> {
        let used: u64 = self.db.query_row("SELECT (SELECT coalesce(sum(size),0) FROM blobs)+(SELECT coalesce(sum(size),0) FROM prepared)",[],|r| crate::state::unsigned(r,0))?;
        let available = self
            .limits
            .max_store
            .saturating_sub(used)
            .min(self.limits.max_output);
        let mut output = 0u64;
        let mut count = 0usize;
        for (layer, diffid) in image
            .manifest
            .layers()
            .iter()
            .zip(image.config.rootfs().diff_ids())
        {
            // First pass validates the complete decompressed stream before any
            // layer effects and collects whiteouts independently of tar order.
            let reader = VerifiedReader {
                reader: self.layer_reader(layer)?,
                hash: Sha256::new(),
                size: 0,
                limit: available,
            };
            let mut archive = tar::Archive::new(reader);
            let mut whiteouts = Vec::new();
            let mut entries = std::collections::BTreeSet::new();
            for entry in archive.entries()?.raw(true) {
                let entry = entry?;
                let path = safe_path(entry.path_bytes().as_ref())?;
                ensure!(entries.insert(path.clone()), "duplicate layer path");
                count += 1;
                ensure!(
                    count <= self.limits.max_entries,
                    "rootfs entry budget exceeded"
                );
                let kind = entry.header().entry_type();
                ensure!(
                    kind.is_file() || kind.is_dir() || kind.is_symlink(),
                    "hardlinks/devices/special entries rejected"
                );
                ensure!(
                    entry.size() <= self.limits.max_entry,
                    "oversized layer entry"
                );
                output = output
                    .checked_add(entry.size())
                    .ok_or_else(|| anyhow::anyhow!("output overflow"))?;
                ensure!(output <= available, "rootfs output budget exceeded");
                if kind.is_symlink() {
                    let target = entry
                        .link_name_bytes()
                        .ok_or_else(|| anyhow::anyhow!("missing link target"))?;
                    safe_target(&path, &target)?;
                }
                if path
                    .file_name()
                    .and_then(|v| v.to_str())
                    .is_some_and(|v| v.starts_with(".wh."))
                {
                    ensure!(kind.is_file() && entry.size() == 0, "malformed whiteout");
                    whiteouts.push(path);
                }
            }
            let mut reader = archive.into_inner();
            let start = reader.size;
            let mut tail = [0u8; 64 * 1024];
            loop {
                let n = reader.read(&mut tail)?;
                if n == 0 {
                    break;
                }
                ensure!(
                    tail[..n].iter().all(|b| *b == 0),
                    "nonzero data after tar terminator"
                );
            }
            ensure!(
                reader.size.saturating_sub(start) >= 512,
                "missing tar end block"
            );
            ensure!(
                format!("sha256:{}", hex::encode(reader.hash.finalize())) == *diffid,
                "DiffID mismatch"
            );
            for path in whiteouts {
                whiteout(root, &path)?;
            }
            let mut archive = tar::Archive::new(self.layer_reader(layer)?);
            for entry in archive.entries()?.raw(true) {
                let mut entry = entry?;
                let path = safe_path(entry.path_bytes().as_ref())?;
                let name = path
                    .file_name()
                    .and_then(|v| v.to_str())
                    .ok_or_else(|| anyhow::anyhow!("invalid name"))?;
                if name.starts_with(".wh.") {
                    continue;
                }
                let parent = parent(root, &path)?;
                let kind = entry.header().entry_type();
                if kind.is_dir() {
                    match parent.symlink_metadata(name) {
                        Ok(meta) if !meta.is_dir() => tree::remove(&parent, name)?,
                        Ok(_) => {}
                        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                        Err(e) => return Err(e.into()),
                    }
                    filesystem::directory(&parent, name)?;
                } else {
                    if parent.symlink_metadata(name).is_ok() {
                        tree::remove(&parent, name)?;
                    }
                    if kind.is_symlink() {
                        let target = entry
                            .link_name_bytes()
                            .ok_or_else(|| anyhow::anyhow!("missing link target"))?;
                        let target = safe_target(&path, &target)?;
                        parent.symlink_contents(target, name)?;
                    } else {
                        let mut file = filesystem::create(&parent, name)?;
                        let n = std::io::copy(&mut entry, &mut file)?;
                        ensure!(n == entry.size(), "truncated layer entry");
                        file.flush()?;
                        let executable = entry.header().mode()? & 0o111 != 0;
                        rustix::fs::fchmod(
                            &file,
                            rustix::fs::Mode::from_raw_mode(if executable { 0o500 } else { 0o400 }),
                        )?;
                        file.sync_all()?;
                    }
                }
                filesystem::sync(&parent)?;
            }
        }
        Ok(output)
    }
}

fn safe_path(bytes: &[u8]) -> Result<std::path::PathBuf> {
    ensure!(
        bytes.len() <= 4096 && !bytes.contains(&0),
        "invalid archive path"
    );
    let text = std::str::from_utf8(bytes)?;
    let path = Path::new(text.trim_end_matches('/'));
    ensure!(
        !path.as_os_str().is_empty() && path.components().count() <= 128,
        "invalid archive path"
    );
    ensure!(
        path.components().all(|c| matches!(c, Component::Normal(_))),
        "absolute/traversing archive path"
    );
    Ok(path.to_owned())
}
fn parent(root: &Dir, path: &Path) -> Result<Dir> {
    let mut dir = root.try_clone()?;
    if let Some(parent) = path.parent() {
        for component in parent.components() {
            let Component::Normal(name) = component else {
                bail!("invalid parent path")
            };
            let name = name
                .to_str()
                .ok_or_else(|| anyhow::anyhow!("non-UTF8 path"))?;
            dir = filesystem::directory(&dir, name)?;
        }
    }
    Ok(dir)
}
pub(super) fn safe_target(path: &Path, bytes: &[u8]) -> Result<String> {
    ensure!(
        !bytes.is_empty() && bytes.len() <= 4096 && !bytes.contains(&0),
        "invalid link target"
    );
    let text = std::str::from_utf8(bytes)?;
    let target = Path::new(text);
    ensure!(!target.is_absolute(), "absolute symlink target rejected");
    let mut depth = path.parent().map_or(0, |p| p.components().count());
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
    Ok(text.to_owned())
}
fn whiteout(root: &Dir, path: &Path) -> Result<()> {
    let dir = parent(root, path)?;
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| anyhow::anyhow!("invalid whiteout"))?;
    if name == ".wh..wh..opq" {
        let names = tree::names(&dir)?;
        for name in names {
            tree::remove(&dir, &name)?;
        }
    } else {
        let target = name
            .strip_prefix(".wh.")
            .ok_or_else(|| anyhow::anyhow!("invalid whiteout"))?;
        ensure!(
            !target.is_empty() && target != "." && target != "..",
            "invalid whiteout target"
        );
        if dir.symlink_metadata(target).is_ok() {
            tree::remove(&dir, target)?;
        }
    }
    Ok(())
}
