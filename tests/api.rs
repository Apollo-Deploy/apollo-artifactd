#![cfg(target_os = "linux")]
use apollo_artifactd::api::call;
use artifactd_protocol::{Action, Request, VERSION, wire};
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
    let request = |id: &str, action| Request {
        version: VERSION,
        operation_id: id.to_owned().try_into().unwrap(),
        action,
    };
    let d: artifactd_protocol::ArtifactDigest =
        format!("sha256:{}", hex::encode(Sha256::digest(b"hello")))
            .parse()
            .unwrap();
    let input = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(input.path(), b"hello").unwrap();
    let import = request(
        "import",
        Action::ImportBlob {
            digest: Some(d.clone()),
            size: 5,
        },
    );
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
        for i in 0..16 {
            let socket = &socket;
            let path = input.path();
            let digest = d.clone();
            tasks.push(scope.spawn(move || {
                let file = std::fs::File::open(path).unwrap();
                let action = request(
                    &format!("duplicate-{i}"),
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
    let calculated = request(
        "calculate",
        Action::ImportBlob {
            digest: None,
            size: 5,
        },
    );
    let fresh_input = std::fs::File::open(input.path()).unwrap();
    let result = call(&socket, &calculated, Some(&fresh_input))
        .unwrap()
        .0
        .result
        .unwrap();
    assert_eq!(result["artifact_digest"], d.as_str());
    assert_eq!(result["size"], 5);
    let conflict = request("import", Action::Gc { max_entries: 1 });
    assert!(call(&socket, &conflict, None).unwrap().0.result.is_err());
    let lease: artifactd_protocol::LeaseId = "consumer".to_owned().try_into().unwrap();
    assert!(
        call(
            &socket,
            &request(
                "lease",
                Action::LeaseCreate {
                    id: lease.clone(),
                    digest: d.clone()
                }
            ),
            None
        )
        .unwrap()
        .0
        .result
        .is_ok()
    );
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
        call(
            &socket,
            &request("release", Action::LeaseRelease { id: lease }),
            None
        )
        .unwrap()
        .0
        .result
        .is_ok()
    );
    assert!(call(&socket, &open, None).unwrap().0.result.is_err());
    let observed = request("observed", Action::Verify { digest: d.clone() });
    assert!(call(&socket, &observed, None).unwrap().0.result.is_ok());
    assert!(
        call(
            &socket,
            &request("gc", Action::Gc { max_entries: 10 }),
            None
        )
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
    assert!(
        call(&socket, &request("doctor", Action::Doctor), None)
            .unwrap()
            .0
            .result
            .is_ok()
    );
}
