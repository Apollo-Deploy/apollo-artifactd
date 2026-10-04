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
                .stderr(Stdio::null())
                .spawn()
                .unwrap(),
        );
        for _ in 0..100 {
            if socket.exists() {
                return (daemon, socket);
            }
            assert!(
                daemon.0.try_wait().unwrap().is_none(),
                "daemon exited before ready"
            );
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
