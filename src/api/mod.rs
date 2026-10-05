mod dispatch;
mod intake;
mod journal;
mod journal_recovery;
mod policy;
mod runtime;
use crate::{Limits, Store, filesystem};
use anyhow::{Result, ensure};
use artifactd_protocol::{Request, Response, VERSION, wire};
use cap_std::fs::MetadataExt;
use rustix::{
    fd::OwnedFd,
    net::{self, AddressFamily, SocketFlags, SocketType},
    process::geteuid,
};
use std::path::Path;

pub fn serve(store: &Path, path: &Path, policy_path: Option<&Path>) -> Result<()> {
    ensure!(
        geteuid().as_raw() != 0,
        "run artifactd as an unprivileged dedicated user"
    );
    let policy = policy::Policy::load(policy_path)?;
    let mut store = Store::open(store, Limits::default())?;
    journal_recovery::audit(&store, true)?;
    let parent = path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("missing socket parent"))?;
    let dir = runtime::open(parent, policy.socket_gid())?;
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
                    && meta.gid() == policy.socket_gid().unwrap_or(meta.gid())
                    && meta.mode() & 0o777 == runtime::socket_mode(policy.socket_gid()),
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
        SocketFlags::CLOEXEC | SocketFlags::NONBLOCK,
        None,
    )?;
    runtime::bind_socket(&dir, path, &listener)?;
    runtime::finalize_socket(&dir, name, policy.socket_gid())?;
    net::listen(&listener, 32)?;
    filesystem::sync(&dir)?;
    let mut intake = intake::Intake::new();
    loop {
        let (socket, caller) = intake.next(&listener, &policy)?;
        if let Err(_error) = handle(&mut store, &socket, &caller, &policy) {
            // Do not log customer input, credentials or registry diagnostics.
            eprintln!("artifactd request rejected");
        }
        ensure!(
            store.db.is_healthy(),
            "database quarantined; restart and reconciliation required"
        );
    }
}

fn handle(
    store: &mut Store,
    socket: &OwnedFd,
    caller: &crate::state::PeerIdentity,
    policy: &policy::Policy,
) -> Result<()> {
    wire::wait(socket, false)?;
    let (bytes, fd) = wire::receive(socket)?;
    let request: Request = serde_json::from_slice(&bytes)?;
    ensure!(request.version == VERSION, "unsupported API version");
    let (result, output) = match policy.authorize_request(&request, caller) {
        Ok(()) => dispatch::execute(store, &request, fd, caller, policy),
        Err(error) => (Err(error.to_string()), None),
    };
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
