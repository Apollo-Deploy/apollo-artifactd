#![cfg(target_os = "linux")]

use artifactd_protocol::{Action, Request, Response, VERSION, client::Client, wire};
use rustix::{
    net::{self, AddressFamily, SocketAddrUnix, SocketFlags, SocketType},
    process::geteuid,
};
use std::{
    io::{Error, ErrorKind, Read, Seek, SeekFrom, Write},
    path::PathBuf,
    sync::mpsc,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

struct Observation {
    request: Option<Request>,
    fd_bytes: Option<Vec<u8>>,
}

struct Fixture {
    _directory: tempfile::TempDir,
    path: PathBuf,
    observed: mpsc::Receiver<Observation>,
    received: Arc<AtomicBool>,
    release: mpsc::Sender<()>,
    worker: thread::JoinHandle<()>,
}

enum Reply {
    Immediate,
    None,
}

fn fixture(reply: Reply) -> Fixture {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("artifactd.sock");
    let listener = net::socket_with(
        AddressFamily::UNIX,
        SocketType::SEQPACKET,
        SocketFlags::CLOEXEC | SocketFlags::NONBLOCK,
        None,
    )
    .unwrap();
    net::bind(&listener, &SocketAddrUnix::new(&path).unwrap()).unwrap();
    net::listen(&listener, 1).unwrap();
    let (send, observed) = mpsc::channel();
    let (release, released) = mpsc::channel();
    let received = Arc::new(AtomicBool::new(false));
    let received_by_worker = received.clone();
    let worker = thread::spawn(move || {
        if wire::wait_for(&listener, false, 1).is_err() {
            send.send(Observation {
                request: None,
                fd_bytes: None,
            })
            .unwrap();
            return;
        }
        let socket = net::accept_with(&listener, SocketFlags::CLOEXEC).unwrap();
        wire::wait_for(&socket, false, 1).unwrap();
        let (bytes, fd) = wire::receive(&socket).unwrap();
        let request: Request = serde_json::from_slice(&bytes).unwrap();
        let operation_id = request.operation_id.clone();
        let fd_bytes = fd.map(|fd| {
            let mut file = std::fs::File::from(fd);
            file.seek(SeekFrom::Start(0)).unwrap();
            let mut bytes = Vec::new();
            file.read_to_end(&mut bytes).unwrap();
            bytes
        });
        send.send(Observation {
            request: Some(request),
            fd_bytes,
        })
        .unwrap();
        received_by_worker.store(true, Ordering::Release);
        if matches!(reply, Reply::Immediate) {
            let response = Response {
                version: VERSION,
                operation_id,
                result: Ok(serde_json::json!({"received": true})),
            };
            wire::send(&socket, &serde_json::to_vec(&response).unwrap(), None).unwrap();
        } else {
            let _ = released.recv_timeout(Duration::from_secs(10));
        }
    });
    Fixture {
        _directory: directory,
        path,
        observed,
        received,
        release,
        worker,
    }
}

fn request() -> Request {
    Request {
        version: VERSION,
        operation_id: "control-test".to_owned().try_into().unwrap(),
        action: Action::ImportBlob {
            digest: None,
            size: 4,
        },
    }
}

#[test]
fn cancellation_after_transmission_preserves_exact_request_and_fd_delivery() {
    let fixture = fixture(Reply::None);
    let mut input = tempfile::NamedTempFile::new().unwrap();
    input.write_all(b"data").unwrap();
    let started = Instant::now();
    let received = fixture.received.clone();
    let release = fixture.release.clone();
    let error = Client::new(&fixture.path, geteuid().as_raw())
        .call_with_control(&request(), Some(input.as_file()), || {
            if received.load(Ordering::Acquire) {
                let _ = release.send(());
                Err(Error::new(ErrorKind::Interrupted, "cancelled"))
            } else {
                Ok(())
            }
        })
        .unwrap_err();
    assert!(started.elapsed() < Duration::from_secs(2));
    assert_eq!(error.kind(), ErrorKind::Interrupted);
    let observed = fixture
        .observed
        .recv_timeout(Duration::from_secs(1))
        .unwrap();
    let observed_request = observed.request.unwrap();
    assert_eq!(observed_request.version, VERSION);
    assert_eq!(
        observed_request.operation_id.as_str(),
        request().operation_id.as_str()
    );
    assert_eq!(
        serde_json::to_value(observed_request.action).unwrap(),
        serde_json::to_value(request().action).unwrap()
    );
    assert_eq!(observed.fd_bytes.as_deref(), Some(b"data".as_slice()));
    fixture.worker.join().unwrap();
}

#[test]
fn cancellation_before_transmission_sends_no_request_or_fd() {
    let fixture = fixture(Reply::None);
    let input = tempfile::NamedTempFile::new().unwrap();
    let error = Client::new(&fixture.path, geteuid().as_raw())
        .call_with_control(&request(), Some(input.as_file()), || {
            Err(Error::new(ErrorKind::Interrupted, "cancelled"))
        })
        .unwrap_err();
    assert_eq!(error.kind(), ErrorKind::Interrupted);
    let observed = fixture
        .observed
        .recv_timeout(Duration::from_secs(2))
        .unwrap();
    assert!(observed.request.is_none());
    assert!(observed.fd_bytes.is_none());
    fixture.worker.join().unwrap();
}

#[test]
fn response_is_delivered_when_server_closes_immediately_after_send() {
    let fixture = fixture(Reply::Immediate);
    let (response, fd) = Client::new(&fixture.path, geteuid().as_raw())
        .call_with_control(&request(), None, || Ok(()))
        .unwrap();
    assert!(fd.is_none());
    assert_eq!(response.result.unwrap()["received"], true);
    let observed = fixture
        .observed
        .recv_timeout(Duration::from_secs(1))
        .unwrap();
    let observed_request = observed.request.unwrap();
    assert_eq!(observed_request.version, VERSION);
    assert_eq!(
        observed_request.operation_id.as_str(),
        request().operation_id.as_str()
    );
    assert_eq!(
        serde_json::to_value(observed_request.action).unwrap(),
        serde_json::to_value(request().action).unwrap()
    );
    assert!(observed.fd_bytes.is_none());
    fixture.worker.join().unwrap();
}

#[test]
fn saturated_socket_backlog_returns_without_blocking_or_delivering_input() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("saturated.sock");
    let listener = net::socket_with(
        AddressFamily::UNIX,
        SocketType::SEQPACKET,
        SocketFlags::CLOEXEC | SocketFlags::NONBLOCK,
        None,
    )
    .unwrap();
    let address = SocketAddrUnix::new(&path).unwrap();
    net::bind(&listener, &address).unwrap();
    net::listen(&listener, 1).unwrap();
    let mut pending = Vec::new();
    let mut saturated = false;
    for _ in 0..32 {
        let socket = net::socket_with(
            AddressFamily::UNIX,
            SocketType::SEQPACKET,
            SocketFlags::CLOEXEC | SocketFlags::NONBLOCK,
            None,
        )
        .unwrap();
        match net::connect(&socket, &address) {
            Ok(()) => pending.push(socket),
            Err(error) if error == rustix::io::Errno::AGAIN => {
                saturated = true;
                break;
            }
            Err(error) => panic!("unexpected backlog setup failure: {error}"),
        }
    }
    assert!(saturated, "fixture did not saturate its socket backlog");
    let (release, released) = mpsc::channel();
    let worker = thread::spawn(move || {
        // Release even a regressed blocking connect so the test fails promptly.
        let _ = released.recv_timeout(Duration::from_secs(2));
        drop(listener);
        drop(pending);
    });
    let mut input = tempfile::NamedTempFile::new().unwrap();
    input.write_all(b"data").unwrap();
    let started = Instant::now();
    let outcome = Client::new(&path, geteuid().as_raw()).call(&request(), Some(input.as_file()));
    let elapsed = started.elapsed();
    let _ = release.send(());
    worker.join().unwrap();
    assert!(
        elapsed < Duration::from_secs(1),
        "saturated connect blocked: {elapsed:?}"
    );
    assert_eq!(outcome.unwrap_err().kind(), ErrorKind::WouldBlock);
}
