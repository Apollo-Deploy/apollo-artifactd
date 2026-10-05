#![cfg(target_os = "linux")]
use apollo_artifactd::api::call;
use artifactd_protocol::{Action, OperationId, Request, VERSION, client, wire};
use daemon::Daemon;
use sha2::{Digest, Sha256};
use std::os::unix::fs::PermissionsExt;
#[path = "support/daemon.rs"]
mod daemon;

#[test]
fn daemon_fd_contract_idempotency_and_replay() {
    let root = tempfile::tempdir().unwrap();
    let runtime = tempfile::tempdir().unwrap();
    for dir in [root.path(), runtime.path()] {
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    let (_daemon, socket) = Daemon::spawn(root.path(), runtime.path());
    let request_id = |operation_id: OperationId, action| Request {
        version: VERSION,
        operation_id,
        action,
    };
    let request = |id: &str, action| request_id(id.to_owned().try_into().unwrap(), action);
    let mutation = |action| request_id(client::allocate(&socket).unwrap(), action);
    let d: artifactd_protocol::ArtifactDigest =
        format!("sha256:{}", hex::encode(Sha256::digest(b"hello")))
            .parse()
            .unwrap();
    let input = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(input.path(), b"hello").unwrap();
    let import = mutation(Action::ImportBlob {
        digest: Some(d.clone()),
        size: 5,
    });
    assert!(
        call(&socket, &import, Some(input.as_file()))
            .unwrap()
            .0
            .result
            .is_ok()
    );
    assert!(
        call(&socket, &import, Some(input.as_file()))
            .unwrap()
            .0
            .result
            .is_ok()
    );
    std::thread::scope(|scope| {
        let mut tasks = Vec::new();
        for _ in 0..16 {
            let socket = &socket;
            let path = input.path();
            let digest = d.clone();
            tasks.push(scope.spawn(move || {
                let file = std::fs::File::open(path).unwrap();
                let action = request_id(
                    client::allocate(socket).unwrap(),
                    Action::ImportBlob {
                        digest: Some(digest),
                        size: 5,
                    },
                );
                assert!(call(socket, &action, Some(&file)).unwrap().0.result.is_ok());
            }));
        }
        for task in tasks {
            task.join().unwrap();
        }
    });
    let calculated = mutation(Action::ImportBlob {
        digest: None,
        size: 5,
    });
    let fresh_input = std::fs::File::open(input.path()).unwrap();
    let result = call(&socket, &calculated, Some(&fresh_input))
        .unwrap()
        .0
        .result
        .unwrap();
    assert_eq!(result["artifact_digest"], d.as_str());
    assert_eq!(result["size"], 5);
    let conflict = request_id(import.operation_id.clone(), Action::Gc { max_entries: 1 });
    assert!(call(&socket, &conflict, None).unwrap().0.result.is_err());
    let lease_result = call(
        &socket,
        &mutation(Action::LeaseCreate {
            digest: d.clone(),
            grantee: None,
        }),
        None,
    )
    .unwrap()
    .0
    .result
    .unwrap();
    let lease: artifactd_protocol::LeaseId = lease_result["lease_id"]
        .as_str()
        .unwrap()
        .to_owned()
        .try_into()
        .unwrap();
    let open = request(
        "open",
        Action::OpenBlob {
            digest: d.clone(),
            lease: lease.clone(),
        },
    );
    let (response, fd) = call(&socket, &open, None).unwrap();
    assert!(response.result.is_ok());
    let fd = fd.unwrap();
    assert!(rustix::io::write(&fd, b"mutation").is_err());
    assert!(
        call(&socket, &mutation(Action::LeaseRelease { id: lease }), None)
            .unwrap()
            .0
            .result
            .is_ok()
    );
    assert!(call(&socket, &open, None).unwrap().0.result.is_err());
    let observed = request("observed", Action::Verify { digest: d.clone() });
    assert!(call(&socket, &observed, None).unwrap().0.result.is_ok());
    assert!(
        call(&socket, &mutation(Action::Gc { max_entries: 10 }), None)
            .unwrap()
            .0
            .result
            .is_ok()
    );
    assert!(call(&socket, &observed, None).unwrap().0.result.is_err());
    // A truncated packet must not consume any descriptor or perform an action.
    let raw = rustix::net::socket_with(
        rustix::net::AddressFamily::UNIX,
        rustix::net::SocketType::SEQPACKET,
        rustix::net::SocketFlags::CLOEXEC,
        None,
    )
    .unwrap();
    rustix::net::connect(&raw, &rustix::net::SocketAddrUnix::new(&socket).unwrap()).unwrap();
    wire::send(&raw, b"{", None).unwrap();
    drop(raw);
    let doctor = mutation(Action::Doctor);
    let facts = call(&socket, &doctor, None).unwrap().0.result.unwrap();
    assert_eq!(facts["inspection_scope"], "operation_execution");
    assert_eq!(facts["database_integrity_checked"], true);
    assert_eq!(facts["database_integrity_clean"], true);
    assert_eq!(facts["database_repaired"], false);
    assert_eq!(facts["live_graphs_verified"], true);
    // Retrying maintenance returns the recorded outcome, not a second repair.
    assert_eq!(
        call(&socket, &doctor, None).unwrap().0.result.unwrap(),
        facts
    );
}
