#![cfg(target_os = "linux")]

use artifactd_protocol::{Action, Request, Response, VERSION, client::Client, wire};
use rustix::{
    net::{self, AddressFamily, SocketAddrUnix, SocketFlags, SocketType},
    process::geteuid,
};
use std::{
    os::unix::{fs::PermissionsExt, process::CommandExt},
    path::PathBuf,
    process::Command,
    sync::mpsc,
    thread,
};

struct Endpoint {
    _dir: tempfile::TempDir,
    path: PathBuf,
    observed: mpsc::Receiver<bool>,
    worker: thread::JoinHandle<()>,
}

fn new_endpoint(respond: bool) -> Endpoint {
    endpoint_with_modes(respond, 0o700, 0o600)
}

fn endpoint_with_modes(respond: bool, directory_mode: u32, socket_mode: u32) -> Endpoint {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("endpoint.sock");
    let listener = net::socket_with(
        AddressFamily::UNIX,
        SocketType::SEQPACKET,
        SocketFlags::CLOEXEC,
        None,
    )
    .unwrap();
    net::bind(&listener, &SocketAddrUnix::new(&path).unwrap()).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(socket_mode)).unwrap();
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(directory_mode)).unwrap();
    net::listen(&listener, 1).unwrap();
    let (send, observed) = mpsc::channel();
    let worker = thread::spawn(move || {
        let socket = net::accept_with(&listener, SocketFlags::CLOEXEC).unwrap();
        let packet = if wire::wait_for(&socket, false, 1).is_ok() {
            wire::receive(&socket).ok()
        } else {
            None
        };
        send.send(packet.is_some()).unwrap();
        if let Some((bytes, _)) = packet
            && respond
        {
            let request: Request = serde_json::from_slice(&bytes).unwrap();
            let response = Response {
                version: VERSION,
                operation_id: request.operation_id,
                result: Ok(serde_json::json!({"ready": true})),
            };
            wire::send(&socket, &serde_json::to_vec(&response).unwrap(), None).unwrap();
        }
    });
    Endpoint {
        _dir: dir,
        path,
        observed,
        worker,
    }
}

fn wrong_uid() -> u32 {
    geteuid().as_raw().checked_add(1).unwrap_or(0)
}

#[test]
fn wrong_uid_rejects_before_request_or_descriptor_send() {
    let endpoint = new_endpoint(true);
    let input = tempfile::NamedTempFile::new().unwrap();
    let request = Request {
        version: VERSION,
        operation_id: "identity-check".to_owned().try_into().unwrap(),
        action: Action::ImportBlob {
            digest: None,
            size: 0,
        },
    };
    let error = Client::new(&endpoint.path, wrong_uid())
        .call(&request, Some(input.as_file()))
        .unwrap_err();
    assert!(error.to_string().contains("unexpected artifactd owner"));
    assert!(!endpoint.observed.recv().unwrap());
    endpoint.worker.join().unwrap();
}

#[test]
fn valid_uid_endpoint_receives_and_answers_public_request() {
    let endpoint = new_endpoint(true);
    let request = Request {
        version: VERSION,
        operation_id: "identity-valid".to_owned().try_into().unwrap(),
        action: Action::Capabilities,
    };
    let response = Client::new(&endpoint.path, geteuid().as_raw())
        .call(&request, None)
        .unwrap()
        .0;
    assert_eq!(response.result.unwrap()["ready"], true);
    assert!(endpoint.observed.recv().unwrap());
    endpoint.worker.join().unwrap();
}

#[test]
fn wrong_uid_rejects_client_allocate_and_cli_autoallocation() {
    let endpoint = new_endpoint(false);
    assert!(Client::new(&endpoint.path, wrong_uid()).allocate().is_err());
    assert!(!endpoint.observed.recv().unwrap());
    endpoint.worker.join().unwrap();

    let endpoint = new_endpoint(false);
    let action = serde_json::to_string(&Action::Unpin {
        id: "cli-identity".to_owned().try_into().unwrap(),
    })
    .unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_apollo-artifactctl"))
        .args([
            "--socket",
            endpoint.path.to_str().unwrap(),
            "--server-uid",
            &wrong_uid().to_string(),
            "--action",
            &action,
        ])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("unexpected artifactd owner"));
    assert!(!endpoint.observed.recv().unwrap());
    endpoint.worker.join().unwrap();
}

#[test]
#[ignore = "requires explicit root execution and nobody (65534) account"]
fn privileged_cross_uid_cli_identity_boundary() {
    assert_eq!(geteuid().as_raw(), 0, "run this ignored case as root");

    // Home directories may forbid the consumer UID from traversing the build tree.
    // Copy only the test CLI into an isolated root-owned, traversable fixture.
    let binary_dir = tempfile::tempdir().unwrap();
    std::fs::set_permissions(binary_dir.path(), std::fs::Permissions::from_mode(0o711)).unwrap();
    let cli = binary_dir.path().join("artifactctl");
    std::fs::copy(env!("CARGO_BIN_EXE_apollo-artifactctl"), &cli).unwrap();
    std::fs::set_permissions(&cli, std::fs::Permissions::from_mode(0o755)).unwrap();

    let endpoint = endpoint_with_modes(true, 0o711, 0o666);
    let output = Command::new(&cli)
        .args([
            "--socket",
            endpoint.path.to_str().unwrap(),
            "--server-uid",
            "0",
            "--action",
            &serde_json::to_string(&Action::Capabilities).unwrap(),
        ])
        .uid(65534)
        .gid(65534)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(endpoint.observed.recv().unwrap());
    endpoint.worker.join().unwrap();

    let endpoint = endpoint_with_modes(false, 0o711, 0o666);
    let output = Command::new(&cli)
        .args([
            "--socket",
            endpoint.path.to_str().unwrap(),
            "--action",
            &serde_json::to_string(&Action::Capabilities).unwrap(),
        ])
        .uid(65534)
        .gid(65534)
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("unexpected artifactd owner"));
    assert!(!endpoint.observed.recv().unwrap());
    endpoint.worker.join().unwrap();
}
