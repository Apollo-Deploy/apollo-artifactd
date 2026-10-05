use crate::filesystem;
use anyhow::{Result, ensure};
use cap_std::fs::Dir;
use rustix::{
    fd::OwnedFd,
    fs::{self, Mode, OFlags},
    net::{self, SocketAddrUnix},
    process::geteuid,
};
use std::{
    os::fd::AsRawFd,
    path::{Path, PathBuf},
};

pub(crate) fn open(path: &Path, socket_gid: Option<u32>) -> Result<Dir> {
    if socket_gid.is_none() {
        return filesystem::private_root(path);
    }
    ensure!(path.is_absolute(), "runtime path must be absolute");
    let fd = fs::open(
        path,
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )?;
    let stat = fs::fstat(&fd)?;
    ensure!(
        fs::FileType::from_raw_mode(stat.st_mode) == fs::FileType::Directory
            && stat.st_uid == geteuid().as_raw()
            && stat.st_gid == socket_gid.unwrap()
            && stat.st_mode & 0o777 == 0o710,
        "runtime directory must be daemon-owned mode 0710 with the configured socket group"
    );
    Ok(Dir::from_std_file(std::fs::File::from(fd)))
}

pub(crate) fn socket_mode(socket_gid: Option<u32>) -> u32 {
    if socket_gid.is_some() { 0o660 } else { 0o600 }
}

pub(crate) fn bind_socket(dir: &Dir, path: &Path, listener: &OwnedFd) -> Result<()> {
    let name = path
        .file_name()
        .ok_or_else(|| anyhow::anyhow!("invalid socket path"))?;
    let anchored = PathBuf::from(format!("/proc/self/fd/{}", dir.as_raw_fd())).join(name);
    net::bind(listener, &SocketAddrUnix::new(&anchored)?)?;
    Ok(())
}

pub(crate) fn finalize_socket(
    dir: &Dir,
    name: &std::ffi::OsStr,
    socket_gid: Option<u32>,
) -> Result<()> {
    if let Some(gid) = socket_gid {
        fs::chownat(
            dir,
            name,
            None,
            Some(rustix::fs::Gid::from_raw(gid)),
            rustix::fs::AtFlags::empty(),
        )?;
    }
    fs::chmodat(
        dir,
        name,
        Mode::from_raw_mode(socket_mode(socket_gid)),
        rustix::fs::AtFlags::empty(),
    )?;
    Ok(())
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::{bind_socket, open};
    use rustix::net::{self, AddressFamily, SocketAddrUnix, SocketFlags, SocketType};
    use std::os::unix::fs::{FileTypeExt, PermissionsExt};

    #[test]
    fn bind_stays_with_captured_runtime_directory_after_path_replacement() {
        let root = tempfile::tempdir().unwrap();
        let configured_runtime = root.path().join("runtime");
        std::fs::create_dir(&configured_runtime).unwrap();
        std::fs::set_permissions(&configured_runtime, std::fs::Permissions::from_mode(0o700))
            .unwrap();

        // Capture the exact runtime directory the production bind must retain.
        let runtime = open(&configured_runtime, None).unwrap();
        let moved_runtime = root.path().join("runtime-captured");
        std::fs::rename(&configured_runtime, &moved_runtime).unwrap();
        std::fs::create_dir(&configured_runtime).unwrap();
        std::fs::set_permissions(&configured_runtime, std::fs::Permissions::from_mode(0o700))
            .unwrap();
        let sentinel = configured_runtime.join("replacement-sentinel");
        std::fs::write(&sentinel, b"must remain untouched").unwrap();

        let listener = net::socket_with(
            AddressFamily::UNIX,
            SocketType::SEQPACKET,
            SocketFlags::CLOEXEC,
            None,
        )
        .unwrap();
        let configured_socket = configured_runtime.join("artifactd.sock");
        // Before descriptor-anchored binding, this call created its socket at
        // configured_socket after the rename, allowing the replacement parent
        // directory to intercept the daemon's listening endpoint.
        bind_socket(&runtime, &configured_socket, &listener).unwrap();

        let captured_socket = moved_runtime.join("artifactd.sock");
        assert!(
            std::fs::symlink_metadata(&captured_socket)
                .unwrap()
                .file_type()
                .is_socket(),
            "socket must be created in the directory captured before rename"
        );
        assert!(
            std::fs::symlink_metadata(&configured_socket).is_err(),
            "replacement pathname must not receive the socket"
        );
        assert_eq!(std::fs::read(&sentinel).unwrap(), b"must remain untouched");

        net::listen(&listener, 1).unwrap();
        let client = net::socket_with(
            AddressFamily::UNIX,
            SocketType::SEQPACKET,
            SocketFlags::CLOEXEC,
            None,
        )
        .unwrap();
        net::connect(&client, &SocketAddrUnix::new(&captured_socket).unwrap()).unwrap();
    }
}
