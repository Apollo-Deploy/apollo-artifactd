mod dispatch;
mod journal;
mod journal_recovery;
use crate::{Limits, Store, filesystem};
use anyhow::{Result, ensure};
use artifactd_protocol::{Request, Response, VERSION, wire};
use cap_std::fs::MetadataExt;
use rustix::{
    fd::OwnedFd,
    net::{self, AddressFamily, SocketAddrUnix, SocketFlags, SocketType},
    process::geteuid,
};
use std::{os::unix::fs::PermissionsExt, path::Path};

pub fn serve(store: &Path, path: &Path) -> Result<()> {
    ensure!(
        geteuid().as_raw() != 0,
        "run artifactd as an unprivileged dedicated user"
    );
    let mut store = Store::open(store, Limits::default())?;
    journal_recovery::audit(&store, true)?;
    let parent = path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("missing socket parent"))?;
    let dir = filesystem::private_root(parent)?;
    let _socket_lock = filesystem::lock(&dir)?;
    let name = path
        .file_name()
        .ok_or_else(|| anyhow::anyhow!("invalid socket path"))?;
    // The parent lock excludes another daemon; only owned private sockets can
    // be unlinked after restart. Foreign entries never get removed.
    match dir.symlink_metadata(name) {
        Ok(meta) => {
            use cap_std::fs::FileTypeExt;
            ensure!(
                meta.file_type().is_socket()
                    && meta.uid() == geteuid().as_raw()
                    && meta.mode() & 0o077 == 0,
                "foreign socket entry"
            );
            dir.remove_file(name)?;
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e.into()),
    }
    let listener = net::socket_with(
        AddressFamily::UNIX,
        SocketType::SEQPACKET,
        SocketFlags::CLOEXEC,
        None,
    )?;
    net::bind(&listener, &SocketAddrUnix::new(path)?)?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    net::listen(&listener, 32)?;
    filesystem::sync(&dir)?;
    loop {
        let socket = net::accept_with(&listener, SocketFlags::CLOEXEC)?;
        if net::sockopt::socket_peercred(&socket)?.uid != geteuid() {
            continue;
        }
        if let Err(_error) = handle(&mut store, &socket) {
            // Do not log customer input, credentials or registry diagnostics.
            eprintln!("artifactd request rejected");
        }
    }
}

fn handle(store: &mut Store, socket: &OwnedFd) -> Result<()> {
    wire::wait(socket, false)?;
    let (bytes, fd) = wire::receive(socket)?;
    let request: Request = serde_json::from_slice(&bytes)?;
    ensure!(request.version == VERSION, "unsupported API version");
    let (result, output) = dispatch::execute(store, &request, fd);
    let response = Response {
        version: VERSION,
        operation_id: request.operation_id,
        result,
    };
    let bytes = serde_json::to_vec(&response)?;
    wire::wait(socket, true)?;
    wire::send(socket, &bytes, output.as_ref())?;
    Ok(())
}

pub use artifactd_protocol::client::call;
