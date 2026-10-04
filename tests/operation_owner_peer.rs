#![cfg(target_os = "linux")]

use artifactd_protocol::{Action, Response};
use rustix::{
    fs::{self, AtFlags, CWD, Gid, Uid},
    process::geteuid,
};
use std::{
    os::unix::{fs::PermissionsExt, process::CommandExt},
    path::Path,
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
    command.uid(uid).gid(gid).output().unwrap()
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
        let output = run_cli(cli, socket, NOBODY, NOGROUP, &Action::Capabilities, None);
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

#[test]
#[ignore = "requires explicit root execution for isolated numeric UID/GID peers"]
fn kernel_peer_gid_binds_operation_owner_across_restart() {
    assert_eq!(geteuid().as_raw(), 0, "run this ignored case as root");
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
    ));
    assert!(replay.result.is_ok());
    drop(daemon);
}
