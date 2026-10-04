#![cfg(target_os = "linux")]

use artifactd_protocol::{Action, ArtifactDigest, Response};
use rustix::{
    fs::{self, AtFlags, CWD, Gid, Uid},
    process::geteuid,
};
use std::{
    os::unix::{fs::PermissionsExt, process::CommandExt},
    path::{Path, PathBuf},
    process::{Child, Command, Output, Stdio},
};

const NOBODY: u32 = 65_534;
const NOGROUP: u32 = 65_534;

struct Daemon(Child);
impl Drop for Daemon {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn chown(path: &Path, uid: u32, gid: u32) {
    fs::chownat(
        CWD,
        path,
        Some(Uid::from_raw(uid)),
        Some(Gid::from_raw(gid)),
        AtFlags::empty(),
    )
    .unwrap();
}

fn run_cli(
    cli: &Path,
    socket: &Path,
    uid: u32,
    gid: u32,
    action: &Action,
    operation_id: Option<&str>,
    input: Option<&Path>,
) -> Output {
    let action = serde_json::to_string(action).unwrap();
    let mut command = Command::new(cli);
    command.args([
        "--socket",
        socket.to_str().unwrap(),
        "--server-uid",
        &uid.to_string(),
        "--action",
        &action,
    ]);
    if let Some(operation_id) = operation_id {
        command.args(["--operation-id", operation_id]);
    }
    if let Some(input) = input {
        command.args(["--input", input.to_str().unwrap()]);
    }
    command.uid(uid).gid(gid).output().unwrap()
}

fn error(output: &Output) -> String {
    serde_json::from_slice::<Response>(&output.stdout)
        .unwrap()
        .result
        .unwrap_err()
}

fn spawn_daemon(binary: &Path, store: &Path, socket: &Path) -> Daemon {
    let child = Command::new(binary)
        .args([
            "--store",
            store.to_str().unwrap(),
            "--socket",
            socket.to_str().unwrap(),
        ])
        .uid(NOBODY)
        .gid(NOGROUP)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    Daemon(child)
}

fn wait_ready(cli: &Path, socket: &Path) -> Output {
    for _ in 0..100 {
        let output = run_cli(
            cli,
            socket,
            NOBODY,
            NOGROUP,
            &Action::Capabilities,
            None,
            None,
        );
        if output.status.success() {
            return output;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    panic!("unprivileged daemon did not become ready");
}

fn response(output: &Output) -> Response {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

fn assert_foreign_reference(cli: &Path, socket: &Path, action: &Action) {
    let output = run_cli(cli, socket, NOBODY, NOGROUP - 1, action, None, None);
    assert!(!output.status.success());
    assert!(error(&output).contains("reference owner mismatch"));
}

fn root_harness() -> (
    tempfile::TempDir,
    PathBuf,
    PathBuf,
    PathBuf,
    PathBuf,
    PathBuf,
) {
    let root = tempfile::tempdir().unwrap();
    std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o711)).unwrap();
    let store = root.path().join("store");
    let runtime = root.path().join("runtime");
    std::fs::create_dir(&store).unwrap();
    std::fs::create_dir(&runtime).unwrap();
    for path in [&store, &runtime] {
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).unwrap();
        chown(path, NOBODY, NOGROUP);
    }
    let daemon_binary = root.path().join("apollo-artifactd");
    let cli_binary = root.path().join("apollo-artifactctl");
    std::fs::copy(env!("CARGO_BIN_EXE_apollo-artifactd"), &daemon_binary).unwrap();
    std::fs::copy(env!("CARGO_BIN_EXE_apollo-artifactctl"), &cli_binary).unwrap();
    for path in [&daemon_binary, &cli_binary] {
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let socket = runtime.join("artifactd.sock");
    (root, store, runtime, daemon_binary, cli_binary, socket)
}

#[test]
#[ignore = "requires explicit root execution for isolated numeric UID/GID peers"]
fn kernel_peer_gid_binds_operation_owner_across_restart() {
    assert_eq!(geteuid().as_raw(), 0, "run this ignored case as root");
    let (_root, store, _runtime, daemon_binary, cli_binary, socket) = root_harness();
    let daemon = spawn_daemon(&daemon_binary, &store, &socket);

    let ready = wait_ready(&cli_binary, &socket);
    assert!(response(&ready).result.is_ok());
    let allocated = response(&run_cli(
        &cli_binary,
        &socket,
        NOBODY,
        NOGROUP,
        &Action::OperationAllocate,
        None,
        None,
    ));
    let token = allocated.result.unwrap()["operation_id"]
        .as_str()
        .unwrap()
        .to_owned();
    let action = Action::Unpin {
        id: "kernel-owner".to_owned().try_into().unwrap(),
    };
    let assert_foreign_rejected = || {
        let foreign = run_cli(
            &cli_binary,
            &socket,
            NOBODY,
            NOGROUP - 1,
            &action,
            Some(&token),
            None,
        );
        assert!(!foreign.status.success());
        let rejected: Response = serde_json::from_slice(&foreign.stdout).unwrap();
        assert!(
            rejected
                .result
                .unwrap_err()
                .contains("operation owner mismatch")
        );
    };
    assert_foreign_rejected();
    let rightful = response(&run_cli(
        &cli_binary,
        &socket,
        NOBODY,
        NOGROUP,
        &action,
        Some(&token),
        None,
    ));
    assert!(rightful.result.is_ok());
    drop(daemon);

    let daemon = spawn_daemon(&daemon_binary, &store, &socket);
    wait_ready(&cli_binary, &socket);
    assert_foreign_rejected();
    let replay = response(&run_cli(
        &cli_binary,
        &socket,
        NOBODY,
        NOGROUP,
        &action,
        Some(&token),
        None,
    ));
    assert!(replay.result.is_ok());
    drop(daemon);
}

#[test]
#[ignore = "requires explicit root execution for isolated numeric UID/GID peers"]
fn kernel_peer_gid_binds_persistent_pin_and_lease_owners() {
    assert_eq!(geteuid().as_raw(), 0, "run this ignored case as root");
    let (root, store, _runtime, daemon_binary, cli_binary, socket) = root_harness();
    let input = root.path().join("input");
    std::fs::write(&input, b"reference-owner").unwrap();
    std::fs::set_permissions(&input, std::fs::Permissions::from_mode(0o644)).unwrap();
    chown(&input, NOBODY, NOGROUP);
    let daemon = spawn_daemon(&daemon_binary, &store, &socket);
    wait_ready(&cli_binary, &socket);
    let imported = response(&run_cli(
        &cli_binary,
        &socket,
        NOBODY,
        NOGROUP,
        &Action::ImportBlob {
            digest: None,
            size: b"reference-owner".len() as u64,
        },
        None,
        Some(&input),
    ));
    let digest: ArtifactDigest =
        serde_json::from_value(imported.result.unwrap()["artifact_digest"].clone()).unwrap();
    let pin: artifactd_protocol::PinId = "kernel-pin".to_owned().try_into().unwrap();
    let lease: artifactd_protocol::LeaseId = "kernel-lease".to_owned().try_into().unwrap();
    let owner = |action: &Action| {
        response(&run_cli(
            &cli_binary,
            &socket,
            NOBODY,
            NOGROUP,
            action,
            None,
            None,
        ))
        .result
        .unwrap()
    };
    let pin_action = Action::Pin {
        id: pin.clone(),
        digest: digest.clone(),
    };
    let lease_action = Action::LeaseCreate {
        id: lease.clone(),
        digest: digest.clone(),
    };
    owner(&pin_action);
    owner(&lease_action);
    let assert_refs_rejected = || {
        for action in [
            pin_action.clone(),
            lease_action.clone(),
            Action::Unpin { id: pin.clone() },
            Action::LeaseRelease { id: lease.clone() },
            Action::OpenBlob {
                digest: digest.clone(),
                lease: lease.clone(),
            },
        ] {
            assert_foreign_reference(&cli_binary, &socket, &action);
        }
    };
    assert_refs_rejected();
    drop(daemon);
    let daemon = spawn_daemon(&daemon_binary, &store, &socket);
    wait_ready(&cli_binary, &socket);
    assert_refs_rejected();
    owner(&Action::Gc { max_entries: 4096 });
    // These reads/retries prove neither unauthorized removal nor GC lost the roots.
    owner(&Action::Verify {
        digest: digest.clone(),
    });
    owner(&pin_action);
    owner(&lease_action);
    owner(&Action::Unpin { id: pin });
    owner(&Action::LeaseRelease { id: lease });
    owner(&Action::Gc { max_entries: 4096 });
    let verify = run_cli(
        &cli_binary,
        &socket,
        NOBODY,
        NOGROUP,
        &Action::Verify { digest },
        None,
        None,
    );
    assert!(!verify.status.success());
    assert!(error(&verify).contains("blob is not admitted"));
    drop(daemon);
}
