use artifactd_protocol::{Action, Response};
use rustix::fs::{self, AtFlags, CWD, Gid, Uid};
use std::{
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::{Child, Command, Output, Stdio},
};

pub const SERVICE_UID: u32 = 65_534;
pub const SERVICE_GID: u32 = 65_534;
pub const SOCKET_GID: u32 = 65_534;

pub struct Daemon(Child);

impl Drop for Daemon {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

pub struct Harness {
    pub root: tempfile::TempDir,
    pub store: PathBuf,
    pub daemon_binary: PathBuf,
    pub cli_binary: PathBuf,
    // Used by the delegated-FD test; the owner-isolation test shares this fixture.
    #[allow(dead_code)]
    pub helper_binary: PathBuf,
    pub socket: PathBuf,
    policy: PathBuf,
}

pub fn chown(path: &Path, uid: u32, gid: u32) {
    fs::chownat(
        CWD,
        path,
        Some(Uid::from_raw(uid)),
        Some(Gid::from_raw(gid)),
        AtFlags::empty(),
    )
    .unwrap();
}

pub fn with_peer_credentials<T>(uid: u32, gid: u32, body: impl FnOnce() -> T + Send + 'static) -> T
where
    T: Send + 'static,
{
    std::thread::spawn(move || {
        rustix::thread::set_thread_groups(&[Gid::from_raw(SOCKET_GID)]).unwrap();
        rustix::thread::set_thread_gid(Gid::from_raw(gid)).unwrap();
        rustix::thread::set_thread_uid(Uid::from_raw(uid)).unwrap();
        body()
    })
    .join()
    .expect("credential-isolated peer thread panicked")
}

pub fn root_harness(peers: &[(u32, u32, &str)]) -> Harness {
    let root = tempfile::tempdir().unwrap();
    std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o711)).unwrap();
    let store = root.path().join("store");
    let runtime = root.path().join("runtime");
    std::fs::create_dir(&store).unwrap();
    std::fs::create_dir(&runtime).unwrap();
    std::fs::set_permissions(&store, std::fs::Permissions::from_mode(0o700)).unwrap();
    chown(&store, SERVICE_UID, SERVICE_GID);
    std::fs::set_permissions(&runtime, std::fs::Permissions::from_mode(0o710)).unwrap();
    chown(&runtime, SERVICE_UID, SOCKET_GID);

    let policy = root.path().join("policy.json");
    let mut policy_peers = vec![serde_json::json!({
        "uid": SERVICE_UID,
        "gid": SERVICE_GID,
        "role": "admin",
    })];
    policy_peers.extend(
        peers
            .iter()
            .map(|(uid, gid, role)| serde_json::json!({ "uid": uid, "gid": gid, "role": role })),
    );
    std::fs::write(
        &policy,
        serde_json::to_vec(&serde_json::json!({
            "socket_gid": SOCKET_GID,
            "peers": policy_peers,
        }))
        .unwrap(),
    )
    .unwrap();
    std::fs::set_permissions(&policy, std::fs::Permissions::from_mode(0o600)).unwrap();
    chown(&policy, SERVICE_UID, SERVICE_GID);

    let daemon_binary = root.path().join("apollo-artifactd");
    let cli_binary = root.path().join("apollo-artifactctl");
    let helper_binary = root.path().join("peer-test-helper");
    std::fs::copy(env!("CARGO_BIN_EXE_apollo-artifactd"), &daemon_binary).unwrap();
    std::fs::copy(env!("CARGO_BIN_EXE_apollo-artifactctl"), &cli_binary).unwrap();
    std::fs::copy(std::env::current_exe().unwrap(), &helper_binary).unwrap();
    for path in [&daemon_binary, &cli_binary, &helper_binary] {
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let socket = runtime.join("artifactd.sock");
    Harness {
        root,
        store,
        daemon_binary,
        cli_binary,
        helper_binary,
        socket,
        policy,
    }
}

impl Harness {
    pub fn spawn_daemon(&self) -> Daemon {
        let daemon_binary = self.daemon_binary.clone();
        let store = self.store.clone();
        let socket = self.socket.clone();
        let policy = self.policy.clone();
        let child = with_peer_credentials(SERVICE_UID, SERVICE_GID, move || {
            Command::new(daemon_binary)
                .args([
                    "--store",
                    store.to_str().unwrap(),
                    "--socket",
                    socket.to_str().unwrap(),
                    "--policy",
                    policy.to_str().unwrap(),
                ])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .unwrap()
        });
        Daemon(child)
    }

    pub fn run_cli(
        &self,
        uid: u32,
        gid: u32,
        action: &Action,
        operation_id: Option<&str>,
        input: Option<&Path>,
    ) -> Output {
        let action = serde_json::to_string(action).unwrap();
        let cli_binary = self.cli_binary.clone();
        let socket = self.socket.clone();
        let input = input.map(Path::to_path_buf);
        let operation_id = operation_id.map(str::to_owned);
        with_peer_credentials(uid, gid, move || {
            let mut command = Command::new(cli_binary);
            command.args([
                "--socket",
                socket.to_str().unwrap(),
                "--server-uid",
                &SERVICE_UID.to_string(),
                "--action",
                &action,
            ]);
            if let Some(operation_id) = operation_id {
                command.args(["--operation-id", &operation_id]);
            }
            if let Some(input) = input {
                command.args(["--input", input.to_str().unwrap()]);
            }
            command.output().unwrap()
        })
    }

    pub fn wait_ready(&self) -> Output {
        for _ in 0..100 {
            let output = self.run_cli(SERVICE_UID, SERVICE_GID, &Action::Capabilities, None, None);
            if output.status.success() {
                return output;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        panic!("unprivileged daemon did not become ready");
    }
}

pub fn response(output: &Output) -> Response {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

pub fn error(output: &Output) -> String {
    serde_json::from_slice::<Response>(&output.stdout)
        .unwrap()
        .result
        .unwrap_err()
}

pub fn allocate_operation(harness: &Harness, uid: u32, gid: u32) -> String {
    let allocated = response(&harness.run_cli(uid, gid, &Action::OperationAllocate, None, None));
    allocated.result.unwrap()["operation_id"]
        .as_str()
        .unwrap()
        .to_owned()
}
