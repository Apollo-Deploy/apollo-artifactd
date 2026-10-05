use artifactd_protocol::{Action, Request, VERSION, client};
use std::{
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    time::Duration,
};

pub struct Daemon(pub Child);

impl Daemon {
    pub fn spawn(store: &Path, runtime: &Path) -> (Self, PathBuf) {
        let socket = runtime.join("artifactd.sock");
        let mut daemon = Self(
            Command::new(env!("CARGO_BIN_EXE_apollo-artifactd"))
                .env_clear()
                .args([
                    "--store",
                    store.to_str().unwrap(),
                    "--socket",
                    socket.to_str().unwrap(),
                ])
                .stdout(Stdio::null())
                .stderr(Stdio::inherit())
                .spawn()
                .unwrap(),
        );
        let probe = Request {
            version: VERSION,
            operation_id: "startup-probe".to_owned().try_into().unwrap(),
            action: Action::Capabilities,
        };
        for _ in 0..6000 {
            assert!(
                daemon.0.try_wait().unwrap().is_none(),
                "daemon exited before ready"
            );
            if client::call(&socket, &probe, None)
                .is_ok_and(|(response, fd)| response.result.is_ok() && fd.is_none())
            {
                return (daemon, socket);
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        let _ = daemon.0.kill();
        let _ = daemon.0.wait();
        panic!("daemon did not become ready")
    }
}

impl Drop for Daemon {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
