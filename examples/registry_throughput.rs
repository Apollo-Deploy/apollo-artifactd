//! Native HTTPS registry qualification. Requires an administrator-provided,
//! isolated registry and a private credential-provider file.
use anyhow::{Context, Result, ensure};
use apollo_artifactd::{
    Limits, Store,
    registry::{Credentials, Registry},
};
use artifactd_protocol::{ArtifactDigest, Platform};
use oci_client::Reference;
use serde_json::json;
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File},
    io::{Read, Seek, SeekFrom, Write},
    os::unix::fs::{MetadataExt, PermissionsExt},
    path::{Path, PathBuf},
    time::Instant,
};
use tar::{Builder, Header};

const LAYER_BYTES: u64 = 32 << 20;
const MAX_ROUNDS: usize = 100;

struct Repeated {
    remaining: u64,
    byte: u8,
}
impl Read for Repeated {
    fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
        let n = out.len().min(self.remaining as usize);
        out[..n].fill(self.byte);
        self.remaining -= n as u64;
        Ok(n)
    }
}
fn digest(reader: &mut impl Read) -> Result<ArtifactDigest> {
    let mut h = Sha256::new();
    let mut buf = [0u8; 65536];
    loop {
        let n = reader.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(format!("sha256:{}", hex::encode(h.finalize())).parse()?)
}
fn private_dir(path: &Path) -> Result<()> {
    let metadata = fs::symlink_metadata(path).context("benchmark directory must already exist")?;
    ensure!(
        metadata.is_dir()
            && metadata.uid() == rustix::process::geteuid().as_raw()
            && metadata.permissions().mode() & 0o077 == 0,
        "benchmark directory must be a private directory"
    );
    Ok(())
}
fn new_private_dir(path: &Path) -> Result<()> {
    fs::create_dir(path)?;
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    Ok(())
}
fn fixture(path: &Path, round: usize, seed: &str) -> Result<(Store, ArtifactDigest)> {
    new_private_dir(path)?;
    let mut store = Store::open(path, Limits::default())?;
    let mut layer = tempfile::tempfile()?;
    let mut header = Header::new_gnu();
    header.set_size(LAYER_BYTES);
    header.set_mode(0o644);
    header.set_cksum();
    {
        let mut archive = Builder::new(&mut layer);
        archive.append_data(
            &mut header,
            format!("payload-{seed}-{round}"),
            Repeated {
                remaining: LAYER_BYTES,
                byte: (round as u8).wrapping_add(0x41),
            },
        )?;
        archive.finish()?;
    }
    layer.flush()?;
    layer.sync_all()?;
    let size = layer.metadata()?.len();
    layer.seek(SeekFrom::Start(0))?;
    let layer_digest = digest(&mut layer)?;
    layer.seek(SeekFrom::Start(0))?;
    store.import_blob(&mut layer, &layer_digest, size)?;
    let config = serde_json::to_vec(
        &json!({"architecture":"amd64","os":"linux","rootfs":{"type":"layers","diff_ids":[layer_digest]}}),
    )?;
    let config_digest = store.import_calculated(&mut &config[..], config.len() as u64)?;
    let manifest = serde_json::to_vec(
        &json!({"schemaVersion":2,"mediaType":"application/vnd.oci.image.manifest.v1+json","config":{"mediaType":"application/vnd.oci.image.config.v1+json","digest":config_digest,"size":config.len()},"layers":[{"mediaType":"application/vnd.oci.image.layer.v1.tar","digest":layer_digest,"size":size}]}),
    )?;
    let manifest_digest = store.import_calculated(&mut &manifest[..], manifest.len() as u64)?;
    let platform = Platform {
        os: "linux".into(),
        architecture: "amd64".into(),
        variant: None,
    };
    store.admit_oci(&manifest_digest, &platform)?;
    Ok((store, manifest_digest))
}
fn percentile(values: &[f64], p: f64) -> f64 {
    let mut v = values.to_vec();
    v.sort_by(f64::total_cmp);
    v[((v.len() - 1) as f64 * p).round() as usize]
}
#[cfg(target_os = "linux")]
fn process() -> serde_json::Value {
    #[cfg(target_os = "linux")]
    {
        let status = fs::read_to_string("/proc/self/status").unwrap_or_default();
        let field = |name: &str| {
            status
                .lines()
                .find_map(|l| l.strip_prefix(name))
                .and_then(|v| v.split_whitespace().next())
                .and_then(|v| v.parse::<u64>().ok())
        };
        let fds = fs::read_dir("/proc/self/fd").map(|d| d.count() as u64).ok();
        let cpu_ticks = fs::read_to_string("/proc/self/stat").ok().and_then(|v| {
            let fields: Vec<_> = v.rsplit_once(") ")?.1.split_whitespace().collect();
            Some(fields.get(11)?.parse::<u64>().ok()? + fields.get(12)?.parse::<u64>().ok()?)
        });
        json!({"cpu_user_system_ticks":cpu_ticks,"rss_kib":field("VmRSS:"),"threads":field("Threads:"),"fds":fds})
    }
}
#[cfg(not(target_os = "linux"))]
fn process() -> serde_json::Value {
    json!({"cpu_user_system_ticks":null,"rss_kib":null,"threads":null,"fds":null})
}
fn main() -> Result<()> {
    let root: PathBuf = std::env::args()
        .nth(1)
        .context("private benchmark directory required")?
        .into();
    private_dir(&root)?;
    let rounds = std::env::var("ARTIFACTD_REGISTRY_ROUNDS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(8usize)
        .clamp(1, MAX_ROUNDS);
    let destination = std::env::var("ARTIFACTD_TEST_REGISTRY_REFERENCE")
        .context("isolated HTTPS registry reference required")?;
    ensure!(
        !destination.contains("@") && !destination.contains("//"),
        "reference must be an unpinned registry destination"
    );
    let credentials_path = std::env::var("ARTIFACTD_TEST_REGISTRY_CREDENTIALS")
        .context("private credential provider required")?;
    let _ = Credentials::read(Some(File::open(&credentials_path)?))?;
    let destination_ref: Reference = destination
        .parse()
        .map_err(|_| anyhow::anyhow!("invalid registry reference"))?;
    let registry = Registry::new()?;
    let run_seed = uuid::Uuid::new_v4().simple().to_string();
    let platform = Platform {
        os: "linux".into(),
        architecture: "amd64".into(),
        variant: None,
    };
    let mut push_times = Vec::with_capacity(rounds);
    let mut pull_times = Vec::with_capacity(rounds);
    let mut process_samples = vec![process()];
    for round in 0..rounds {
        let source_path = root.join(format!("source-{round:03}"));
        let target_path = root.join(format!("target-{round:03}"));
        let (source, manifest) = fixture(&source_path, round, &run_seed)?;
        new_private_dir(&target_path)?;
        let mut target = Store::open(&target_path, Limits::default())?;
        let started = Instant::now();
        let receipt = registry.push(
            &source,
            &manifest,
            &destination,
            Credentials::read(Some(File::open(&credentials_path)?))?,
        )?;
        push_times.push(started.elapsed().as_secs_f64());
        ensure!(
            receipt.get("verified").and_then(|v| v.as_bool()) == Some(true),
            "registry did not report verified push"
        );
        process_samples.push(process());
        let pinned = destination_ref.clone_with_digest(manifest.as_str().to_owned());
        let started = Instant::now();
        let pulled = registry.pull(
            &mut target,
            &pinned.to_string(),
            &platform,
            Credentials::read(Some(File::open(&credentials_path)?))?,
        )?;
        pull_times.push(started.elapsed().as_secs_f64());
        ensure!(
            pulled.get("artifact_digest").and_then(|v| v.as_str()) == Some(manifest.as_str()),
            "registry pull returned an unexpected artifact digest"
        );
        ensure!(
            pulled.get("manifest_digest").and_then(|v| v.as_str()) == Some(manifest.as_str()),
            "registry pull returned an unexpected manifest digest"
        );
        ensure!(
            pulled.get("platform") == Some(&serde_json::to_value(&platform)?),
            "registry pull returned an unexpected platform"
        );
        let image = target.resolve(&manifest, &platform)?;
        ensure!(
            image.digest == manifest,
            "pulled graph did not resolve to the manifest"
        );
        let _: File = target.open_blob(&image.manifest.config().digest().to_string().parse()?)?;
        for layer in image.manifest.layers() {
            let _: File = target.open_blob(&layer.digest().to_string().parse()?)?;
        }
        process_samples.push(process());
    }
    let mib = LAYER_BYTES as f64 / (1024.0 * 1024.0);
    let summary = |times: &[f64]| json!({"p50_seconds":percentile(times,0.50),"p95_seconds":percentile(times,0.95),"p99_seconds":percentile(times,0.99),"total_mib":mib * times.len() as f64,"aggregate_mib_per_second":mib * times.len() as f64 / times.iter().sum::<f64>()});
    let peak_rss = process_samples
        .iter()
        .filter_map(|v| v.get("rss_kib").and_then(|n| n.as_u64()))
        .max();
    let peak_fds = process_samples
        .iter()
        .filter_map(|v| v.get("fds").and_then(|n| n.as_u64()))
        .max();
    println!(
        "{}",
        json!({"fixture":"one unique 32MiB uncompressed layer per round","rounds":rounds,"fresh_target_stores":true,"verification_and_fsync":true,"push":summary(&push_times),"pull":summary(&pull_times),"process_baseline":process_samples.first(),"process_final":process_samples.last(),"process_peak_rss_kib":peak_rss,"process_peak_fds":peak_fds})
    );
    Ok(())
}
