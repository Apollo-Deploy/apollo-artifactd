#[cfg(target_os = "linux")]
mod linux {
    //! Public Unix-socket API churn qualification. Run with daemon/store/socket paths.
    use anyhow::{Context, Result, ensure};
    use artifactd_protocol::client::Client;
    use artifactd_protocol::{
        Action, ArtifactDigest, Platform, PreparedArtifactId, Request, VERSION,
    };
    use sha2::{Digest, Sha256};
    use std::{
        fs::File,
        io::{Read, Seek, SeekFrom, Write},
        os::fd::OwnedFd,
        path::{Path, PathBuf},
        process::{Child, Command, Stdio},
        time::Instant,
    };

    struct Daemon(Child);
    impl Drop for Daemon {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }

    fn digest(bytes: &[u8]) -> ArtifactDigest {
        format!("sha256:{}", hex::encode(Sha256::digest(bytes)))
            .parse()
            .unwrap()
    }
    fn platform() -> Platform {
        Platform {
            os: "linux".into(),
            architecture: if cfg!(target_arch = "aarch64") {
                "arm64".into()
            } else {
                "amd64".into()
            },
            variant: None,
        }
    }
    fn request(client: &Client, action: Action, input: Option<&File>) -> Result<serde_json::Value> {
        let operation_id = client.allocate()?;
        let request = Request {
            version: VERSION,
            operation_id,
            action,
        };
        let (response, fd) = client.call(&request, input)?;
        ensure!(fd.is_none(), "unexpected FD on non-FD operation");
        response.result.map_err(anyhow::Error::msg)
    }
    fn request_fd(
        client: &Client,
        action: Action,
        input: Option<&File>,
    ) -> Result<(serde_json::Value, OwnedFd)> {
        let operation_id = client.allocate()?;
        let request = Request {
            version: VERSION,
            operation_id,
            action,
        };
        let (response, fd) = client.call(&request, input)?;
        Ok((
            response.result.map_err(anyhow::Error::msg)?,
            fd.context("missing returned FD")?,
        ))
    }
    fn temp_file(bytes: &[u8]) -> Result<File> {
        let mut file = tempfile::tempfile()?;
        file.write_all(bytes)?;
        file.seek(SeekFrom::Start(0))?;
        Ok(file)
    }
    fn bounded_entries(path: &Path) -> Result<usize> {
        let mut count = 0usize;
        for entry in std::fs::read_dir(path).with_context(|| format!("read {}", path.display()))? {
            let _ = entry?;
            count += 1;
            ensure!(
                count <= 10_000,
                "unbounded entries under {}",
                path.display()
            );
        }
        Ok(count)
    }
    fn bounded_tree_entries(path: &Path, seen: &mut usize) -> Result<usize> {
        let mut count = 0;
        for entry in std::fs::read_dir(path).with_context(|| format!("read {}", path.display()))? {
            let entry = entry?;
            *seen += 1;
            ensure!(
                *seen <= 10_000,
                "unbounded entries under {}",
                path.display()
            );
            let ty = entry.file_type()?;
            count += 1;
            if ty.is_dir() {
                count += bounded_tree_entries(&entry.path(), seen)?;
            }
        }
        Ok(count)
    }
    mod metrics {
        include!("api_churn/metrics.rs");
    }
    mod oci {
        include!("api_churn/oci.rs");
    }
    mod cycles {
        include!("api_churn/cycles.rs");
    }

    fn spawn(daemon: &str, store: &Path, socket: &Path) -> Result<Daemon> {
        let mut child = Command::new(daemon)
            .args([
                "--store",
                store.to_str().unwrap(),
                "--socket",
                socket.to_str().unwrap(),
            ])
            .stdout(Stdio::null())
            .stderr(Stdio::inherit())
            .spawn()?;
        let pid = child.id();
        let client = Client::new(socket, rustix::process::geteuid().as_raw());
        for _ in 0..600 {
            if child.try_wait()?.is_some() {
                anyhow::bail!("daemon exited before readiness");
            }
            if client
                .call(
                    &Request {
                        version: VERSION,
                        operation_id: "probe".to_owned().try_into().unwrap(),
                        action: Action::Capabilities,
                    },
                    None,
                )
                .is_ok()
            {
                return Ok(Daemon(child));
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        anyhow::bail!("daemon did not become ready after 6 seconds (pid {pid})")
    }

    pub fn main() -> Result<()> {
        let args: Vec<_> = std::env::args().collect();
        ensure!(
            args.len() >= 4,
            "usage: api_churn <daemon> <store> <socket> [blob_cycles] [oci_cycles]"
        );
        let daemon = &args[1];
        let store = PathBuf::from(&args[2]);
        let socket = PathBuf::from(&args[3]);
        let blobs: u64 = args.get(4).map_or(Ok(100_000), |v| v.parse())?;
        let oci: u64 = args.get(5).map_or(Ok(20_000), |v| v.parse())?;
        ensure!(blobs <= 100_000, "blob_cycles must be <= 100000");
        ensure!(oci <= 20_000, "oci_cycles must be <= 20000");
        std::fs::create_dir_all(&store)?;
        if let Some(parent) = socket.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut child = spawn(daemon, &store, &socket)?;
        let client = Client::new(&socket, rustix::process::geteuid().as_raw());
        let mut stats = cycles::run(&client, &mut child, daemon, &store, &socket, blobs, oci)?;
        let pre_final_restart_resources = metrics::resources(child.0.id(), &store)?;
        child.0.kill()?;
        child.0.wait()?;
        child = spawn(daemon, &store, &socket)?;
        let post_restart_resources = metrics::resources(child.0.id(), &store)?;
        let status = request(&client, Action::Status, None)?;
        let final_resources = metrics::resources(child.0.id(), &store)?;
        let cpu_delta = final_resources["cpu_ticks"]
            .as_u64()
            .unwrap_or(0)
            .saturating_sub(post_restart_resources["cpu_ticks"].as_u64().unwrap_or(0));
        println!(
            "{}",
            serde_json::json!({
                "blob_operations":blobs,
                "oci_lifecycles":oci,
                "bytes": {"blob": stats.total_blob_bytes, "oci_archives": stats.total_oci_bytes},
                "elapsed_us": stats.elapsed_us,
                "throughput_bytes_per_sec": ((stats.total_blob_bytes + stats.total_oci_bytes) as f64 * 1_000_000.0 / stats.elapsed_us.max(1) as f64),
                "blob_latency":metrics::samples(&mut stats.blob_latency),
                "oci_latency":metrics::samples(&mut stats.oci_latency),
                "operation_latency": {
                    "import_blob": metrics::samples(&mut stats.import_blob_latency),
                    "import_archive": metrics::samples(&mut stats.import_archive_latency),
                    "prepare": metrics::samples(&mut stats.prepare_latency),
                    "gc": metrics::samples(&mut stats.gc_latency)
                },
                "baseline":stats.baseline.clone(),
                "post_restart_resources":post_restart_resources,
                "final_resources":final_resources,
                "cpu_ticks_after_restart":cpu_delta,
                "cpu_ticks_segments":stats.cpu_segments.clone(),
                "pre_first_restart_resources":stats.pre_restart_resources.clone(),
                "cpu_ticks_workload_before_first_restart": stats.cpu_segments.first().copied().unwrap_or(0).saturating_sub(stats.baseline["cpu_ticks"].as_u64().unwrap_or(0)),
                "cpu_ticks_cycles_after_restart": pre_final_restart_resources["cpu_ticks"].as_u64().unwrap_or(0).saturating_sub(stats.cpu_segments.get(1).copied().unwrap_or(0)),
                "status":status
            })
        );
        child.0.kill()?;
        child.0.wait()?;
        let dir = cap_std::fs::Dir::open_ambient_dir(&store, cap_std::ambient_authority())?;
        let state = apollo_artifactd::state::State::open(&dir)?;
        for table in [
            "blobs", "imports", "roots", "edges", "pins", "leases", "prepared", "gc", "gc_marks",
            "registry",
        ] {
            ensure!(state.count(table)? == 0, "final inventory retained {table}");
        }
        ensure!(
            state.count("operations")? <= 4096,
            "operation journal exceeded retention bound"
        );
        let mut filesystem_entries = 0;
        let blobs_entries = bounded_tree_entries(&store.join("blobs"), &mut filesystem_entries)?;
        let temp_entries = bounded_tree_entries(&store.join("temp"), &mut filesystem_entries)?;
        let prepared_entries =
            bounded_tree_entries(&store.join("prepared"), &mut filesystem_entries)?;
        ensure!(blobs_entries == 0, "final blobs filesystem not empty");
        ensure!(temp_entries == 0, "final temp filesystem not empty");
        ensure!(prepared_entries == 0, "final prepared filesystem not empty");
        let inventory = serde_json::json!({
            "tables": {
                "blobs": state.count("blobs")?,
                "imports": state.count("imports")?,
                "roots": state.count("roots")?,
                "edges": state.count("edges")?,
                "pins": state.count("pins")?,
                "leases": state.count("leases")?,
                "prepared": state.count("prepared")?,
                "gc": state.count("gc")?,
                "gc_marks": state.count("gc_marks")?,
                "registry": state.count("registry")?,
                "operations": state.count("operations")?
            },
            "store_entries": bounded_entries(&store)?,
            "blobs_entries": blobs_entries,
            "temp_entries": temp_entries,
            "prepared_entries": prepared_entries
        });
        println!("{}", serde_json::json!({"final_inventory": inventory}));
        Ok(())
    }
}

#[cfg(target_os = "linux")]
fn main() -> anyhow::Result<()> {
    linux::main()
}

#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("api_churn requires Linux artifactd protocol support");
}
