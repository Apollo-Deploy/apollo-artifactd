//! Descriptor-relative operations under a private, exclusively owned store.
use anyhow::{Context, Result, ensure};
use cap_std::fs::Dir;
use rustix::{
    fd::OwnedFd,
    fs::{self, Mode, OFlags},
    process::geteuid,
};
use std::{fs::File, os::unix::fs::MetadataExt, path::Path};

pub fn private_root(path: &Path) -> Result<Dir> {
    ensure!(path.is_absolute(), "store path must be absolute");
    // Provisioning is administrative; never create or chmod an existing root.
    let fd = fs::open(
        path,
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )?;
    private_fd(&fd, true)?;
    Ok(Dir::from_std_file(File::from(fd)))
}

pub fn private_fd(fd: &OwnedFd, directory: bool) -> Result<()> {
    let stat = fs::fstat(fd)?;
    ensure!(
        stat.st_uid == geteuid().as_raw() && stat.st_mode & 0o077 == 0,
        "foreign or non-private inode"
    );
    let expected = if directory {
        fs::FileType::Directory
    } else {
        fs::FileType::RegularFile
    };
    ensure!(
        fs::FileType::from_raw_mode(stat.st_mode) == expected,
        "wrong inode type"
    );
    if !directory {
        ensure!(stat.st_nlink == 1, "multiply linked inode");
    }
    Ok(())
}

pub fn directory(parent: &Dir, name: &str) -> Result<Dir> {
    match fs::mkdirat(parent, name, Mode::from_raw_mode(0o700)) {
        Ok(()) => sync(parent)?,
        Err(rustix::io::Errno::EXIST) => {}
        Err(e) => return Err(e.into()),
    }
    let fd = fs::openat(
        parent,
        name,
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )?;
    private_fd(&fd, true)?;
    Ok(Dir::from_std_file(File::from(fd)))
}

pub fn create(dir: &Dir, name: &str) -> Result<File> {
    let fd = fs::openat(
        dir,
        name,
        OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::from_raw_mode(0o600),
    )?;
    private_fd(&fd, false)?;
    Ok(File::from(fd))
}

pub fn read(dir: &Dir, name: &str) -> Result<File> {
    let fd = fs::openat(
        dir,
        name,
        OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NONBLOCK,
        Mode::empty(),
    )?;
    private_fd(&fd, false)?;
    let file = File::from(fd);
    ensure!(
        file.metadata()?.mode() & 0o222 == 0,
        "published content is writable"
    );
    Ok(file)
}

pub fn sync(dir: &Dir) -> Result<()> {
    // cap-std may hold an O_PATH capability on Linux, which cannot be fsynced.
    let fd = fs::openat(
        dir,
        ".",
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )?;
    fs::fsync(&fd).context("sync directory")
}

pub fn lock(root: &Dir) -> Result<File> {
    let fd = fs::openat(
        root,
        "owner.lock",
        OFlags::RDWR | OFlags::CREATE | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::from_raw_mode(0o600),
    )?;
    private_fd(&fd, false)?;
    fs::flock(&fd, fs::FlockOperation::NonBlockingLockExclusive)
        .context("another artifactd owns this store")?;
    Ok(File::from(fd))
}

pub fn owned_remove(dir: &Dir, name: &str) -> Result<()> {
    let fd = fs::openat(
        dir,
        name,
        OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NONBLOCK,
        Mode::empty(),
    )?;
    private_fd(&fd, false)?;
    fs::unlinkat(dir, name, fs::AtFlags::empty())?;
    sync(dir)
}

pub fn readonly(file: &File) -> Result<()> {
    fs::fchmod(file, Mode::from_raw_mode(0o400))?;
    file.sync_all()?;
    Ok(())
}
