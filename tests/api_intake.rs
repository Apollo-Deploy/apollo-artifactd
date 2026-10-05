#![cfg(target_os = "linux")]

#[path = "support/daemon.rs"]
mod daemon;

use artifactd_protocol::{Action, Request, VERSION, client};
use rustix::net::{self, AddressFamily, SocketAddrUnix, SocketFlags, SocketType};
use std::{
    fs,
    os::unix::fs::PermissionsExt,
    time::{Duration, Instant},
};

#[test]
fn silent_authorized_connections_do_not_delay_ready_requests() {
    let store = tempfile::tempdir().unwrap();
    let runtime = tempfile::tempdir().unwrap();
    for path in [store.path(), runtime.path()] {
        fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
    }
    let (_daemon, socket) = daemon::Daemon::spawn(store.path(), runtime.path());
    let mut idle = Vec::new();
    for _ in 0..2 {
        let fd = net::socket_with(
            AddressFamily::UNIX,
            SocketType::SEQPACKET,
            SocketFlags::CLOEXEC,
            None,
        )
        .unwrap();
        net::connect(&fd, &SocketAddrUnix::new(&socket).unwrap()).unwrap();
        idle.push(fd);
    }
    // The request is queued behind both idle connections at the actual socket.
    // Old serial admission waits for their first packets (30 seconds each).
    let started = Instant::now();
    let client = client::Client::new(&socket, rustix::process::geteuid().as_raw());
    let (response, fd) = client
        .call_with_control(
            &Request {
                version: VERSION,
                operation_id: "ready-behind-idle".to_owned().try_into().unwrap(),
                action: Action::Status,
            },
            None,
            || {
                if started.elapsed() >= Duration::from_secs(2) {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::TimedOut,
                        "idle authorized clients blocked ready status",
                    ));
                }
                Ok(())
            },
        )
        .unwrap();
    assert!(response.result.is_ok());
    assert!(fd.is_none());
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "idle clients delayed status: {:?}",
        started.elapsed()
    );
    drop(idle);
}
