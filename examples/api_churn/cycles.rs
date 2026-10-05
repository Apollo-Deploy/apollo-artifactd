use super::*;

pub struct RunStats {
    pub baseline: serde_json::Value,
    pub blob_latency: Vec<u128>,
    pub oci_latency: Vec<u128>,
    pub import_blob_latency: Vec<u128>,
    pub import_archive_latency: Vec<u128>,
    pub prepare_latency: Vec<u128>,
    pub gc_latency: Vec<u128>,
    pub total_blob_bytes: u64,
    pub total_oci_bytes: u64,
    pub elapsed_us: u128,
    pub cpu_segments: Vec<u64>,
    pub pre_restart_resources: Option<serde_json::Value>,
}

pub fn run(
    client: &Client,
    child: &mut Daemon,
    daemon: &str,
    store: &Path,
    socket: &Path,
    blobs: u64,
    oci: u64,
) -> Result<RunStats> {
    let baseline = metrics::resources(child.0.id(), store)?;
    let mut blob_latency = Vec::with_capacity(blobs.min(100_000) as usize);
    let mut import_blob_latency = Vec::with_capacity(blobs.min(100_000) as usize);
    let mut import_archive_latency = Vec::with_capacity(oci.min(20_000) as usize);
    let mut gc_latency = Vec::with_capacity((blobs + oci).min(120_000) as usize);
    let mut total_blob_bytes = 0u64;
    let mut total_oci_bytes = 0u64;
    let run_started = Instant::now();
    let mut cpu_segments = Vec::new();
    let mut pre_restart_resources = None;
    for i in 0..blobs {
        let bytes = format!("api-blob-{i}").into_bytes();
        let expected = digest(&bytes);
        let mut input = temp_file(&bytes)?;
        let now = Instant::now();
        total_blob_bytes += bytes.len() as u64;
        let import_request = Request {
            version: VERSION,
            operation_id: client.allocate()?,
            action: Action::ImportBlob {
                digest: Some(expected.clone()),
                size: bytes.len() as u64,
            },
        };
        let import_started = Instant::now();
        let (import_response, import_fd) = client.call(&import_request, Some(&input))?;
        if import_blob_latency.len() < 100_000 {
            import_blob_latency.push(import_started.elapsed().as_micros());
        }
        ensure!(import_fd.is_none(), "unexpected FD on blob import");
        let imported = match &import_response.result {
            Ok(value) => value,
            Err(error) => anyhow::bail!("blob import failed: {error}"),
        };
        ensure!(
            imported["artifact_digest"].as_str() == Some(expected.as_str()),
            "import returned wrong digest"
        );
        ensure!(
            imported["size"].as_u64() == Some(bytes.len() as u64),
            "import returned wrong size"
        );
        if i == 0 {
            input.seek(SeekFrom::Start(0))?;
            let (replay, _) = client.call(&import_request, Some(&input))?;
            ensure!(
                replay.result == import_response.result,
                "completed import replay changed result"
            );
        }
        let lease: artifactd_protocol::LeaseId = request(
            client,
            Action::LeaseCreate {
                digest: expected.clone(),
                grantee: None,
            },
            None,
        )?["lease_id"]
            .as_str()
            .unwrap()
            .to_owned()
            .try_into()
            .unwrap();
        let (_, fd) = request_fd(
            client,
            Action::OpenBlob {
                digest: expected.clone(),
                lease: lease.clone(),
            },
            None,
        )?;
        let mut returned = File::from(fd);
        let mut actual = Vec::new();
        returned.read_to_end(&mut actual)?;
        ensure!(actual == bytes, "blob FD payload mismatch");
        drop(returned);
        request(client, Action::LeaseRelease { id: lease }, None)?;
        let pin: artifactd_protocol::PinId = format!("blob-pin-{i}").try_into().unwrap();
        request(
            client,
            Action::Pin {
                id: pin.clone(),
                digest: expected,
            },
            None,
        )?;
        request(client, Action::Unpin { id: pin }, None)?;
        let gc_started = Instant::now();
        request(client, Action::Gc { max_entries: 4096 }, None)?;
        if gc_latency.len() < 120_000 {
            gc_latency.push(gc_started.elapsed().as_micros());
        }
        if blob_latency.len() < 100_000 {
            blob_latency.push(now.elapsed().as_micros());
        }
        input.seek(SeekFrom::Start(0))?;
        if i % 10_000 == 0 {
            println!(
                "{}",
                serde_json::json!({"blob_iteration":i,"resources":metrics::resources(child.0.id(), store)?})
            );
        }
    }
    let mut oci_latency = Vec::with_capacity(oci.min(20_000) as usize);
    let mut prepare_latency = Vec::with_capacity(oci.min(20_000) as usize);
    for i in 0..oci {
        let (archive, expected_manifest) = oci::archive(i)?;
        total_oci_bytes += archive.len() as u64;
        let input = temp_file(&archive)?;
        let pin: artifactd_protocol::PinId = format!("oci-pin-{i}").try_into().unwrap();
        let now = Instant::now();
        let import_started = Instant::now();
        let facts = request(
            client,
            Action::ImportOciArchive {
                platform: platform(),
                pin: Some(pin.clone()),
            },
            Some(&input),
        )?;
        if import_archive_latency.len() < 20_000 {
            import_archive_latency.push(import_started.elapsed().as_micros());
        }
        let index_digest: ArtifactDigest = facts["artifact_digest"]
            .as_str()
            .context("missing manifest digest")?
            .parse()?;
        let resolved = request(
            client,
            Action::Resolve {
                digest: index_digest.clone(),
                platform: platform(),
            },
            None,
        )?;
        let manifest: ArtifactDigest = resolved["manifest_digest"]
            .as_str()
            .context("missing resolved manifest digest")?
            .parse()?;
        ensure!(
            manifest == expected_manifest,
            "resolve returned unexpected manifest"
        );
        let lease: artifactd_protocol::LeaseId = request(
            client,
            Action::LeaseCreate {
                digest: index_digest,
                grantee: None,
            },
            None,
        )?["lease_id"]
            .as_str()
            .unwrap()
            .to_owned()
            .try_into()
            .unwrap();
        let prepare_started = Instant::now();
        let prepared = request(
            client,
            Action::Prepare {
                digest: manifest.clone(),
                platform: platform(),
            },
            None,
        )?;
        if prepare_latency.len() < 20_000 {
            prepare_latency.push(prepare_started.elapsed().as_micros());
        }
        let prepared_id: PreparedArtifactId = prepared["prepared_artifact_id"]
            .as_str()
            .context("missing prepared ID")?
            .to_owned()
            .try_into()
            .unwrap();
        let (_, fd) = request_fd(
            client,
            Action::OpenPrepared {
                id: prepared_id.clone(),
                lease: lease.clone(),
            },
            None,
        )?;
        let directory = cap_std::fs::Dir::from_std_file(File::from(fd));
        let mut payload = Vec::new();
        directory.open("payload")?.read_to_end(&mut payload)?;
        ensure!(
            payload == format!("payload-{i}").as_bytes(),
            "prepared payload mismatch"
        );
        drop(directory);
        if i == 0 {
            let before_restart = metrics::resources(child.0.id(), store)?;
            cpu_segments.push(
                before_restart["cpu_ticks"]
                    .as_u64()
                    .context("missing CPU ticks before restart")?,
            );
            pre_restart_resources = Some(before_restart);
            child.0.kill()?;
            child.0.wait()?;
            *child = spawn(daemon, store, socket)?;
            cpu_segments.push(
                metrics::resources(child.0.id(), store)?["cpu_ticks"]
                    .as_u64()
                    .context("missing CPU ticks after restart")?,
            );
            let (_, replay_fd) = request_fd(
                client,
                Action::OpenPrepared {
                    id: prepared_id.clone(),
                    lease: lease.clone(),
                },
                None,
            )?;
            let replay_dir = cap_std::fs::Dir::from_std_file(File::from(replay_fd));
            let mut replay_payload = Vec::new();
            replay_dir
                .open("payload")?
                .read_to_end(&mut replay_payload)?;
            ensure!(
                replay_payload == format!("payload-{i}").as_bytes(),
                "restart payload mismatch"
            );
            drop(replay_dir);
        }
        request(client, Action::LeaseRelease { id: lease }, None)?;
        request(client, Action::Unpin { id: pin }, None)?;
        let gc_started = Instant::now();
        request(client, Action::Gc { max_entries: 4096 }, None)?;
        if gc_latency.len() < 120_000 {
            gc_latency.push(gc_started.elapsed().as_micros());
        }
        if oci_latency.len() < 20_000 {
            oci_latency.push(now.elapsed().as_micros());
        }
        if i % 2_000 == 0 {
            println!(
                "{}",
                serde_json::json!({"oci_iteration":i,"resources":metrics::resources(child.0.id(), store)?})
            );
        }
    }
    Ok(RunStats {
        baseline,
        blob_latency,
        oci_latency,
        import_blob_latency,
        import_archive_latency,
        prepare_latency,
        gc_latency,
        total_blob_bytes,
        total_oci_bytes,
        elapsed_us: run_started.elapsed().as_micros(),
        cpu_segments,
        pre_restart_resources,
    })
}
