//! Native qualification runner; owns only the administrator-provisioned store.
use anyhow::{Result, ensure};
use apollo_artifactd::{Limits, Store};
use artifactd_protocol::{ArtifactDigest, Platform};
use sha2::{Digest, Sha256};
use std::{io::Cursor, path::PathBuf, time::Instant};

fn import(store: &mut Store, bytes: &[u8]) -> Result<ArtifactDigest> {
    let digest = format!("sha256:{}", hex::encode(Sha256::digest(bytes))).parse()?;
    store.import_blob(&mut Cursor::new(bytes), &digest, bytes.len() as u64)?;
    Ok(digest)
}
fn resources() -> serde_json::Value {
    #[cfg(target_os = "linux")]
    {
        let status = std::fs::read_to_string("/proc/self/status").unwrap_or_default();
        let selected: Vec<_> = status
            .lines()
            .filter(|line| {
                ["VmRSS:", "VmHWM:", "Threads:"]
                    .iter()
                    .any(|k| line.starts_with(k))
            })
            .collect();
        let fds = std::fs::read_dir("/proc/self/fd")
            .map(|it| it.count())
            .unwrap_or(0);
        serde_json::json!({"status":selected,"fds":fds})
    }
    #[cfg(not(target_os = "linux"))]
    {
        serde_json::json!({"native_linux":false})
    }
}
fn distribution(mut samples: Vec<u128>) -> serde_json::Value {
    samples.sort();
    let value = |n: usize| {
        samples
            .get(samples.len().saturating_sub(1) * n / 100)
            .copied()
            .unwrap_or(0)
    };
    serde_json::json!({"p50_us":value(50),"p95_us":value(95),"p99_us":value(99)})
}
fn main() -> Result<()> {
    let path: PathBuf = std::env::args()
        .nth(1)
        .ok_or_else(|| anyhow::anyhow!("private store path required"))?
        .into();
    let mut store = Store::open(&path, Limits::default())?;
    let baseline = resources();
    let start = Instant::now();
    let mut latency = Vec::with_capacity(100_000);
    for i in 0u64..100_000 {
        let now = Instant::now();
        let digest = import(&mut store, &i.to_le_bytes())?;
        store.pin("churn", &digest)?;
        store.unpin("churn")?;
        ensure!(store.gc(32)? == 1, "blob churn collection failed");
        latency.push(now.elapsed().as_micros());
        if i % 10_000 == 0 {
            println!(
                "{}",
                serde_json::json!({"blob_iteration":i,"resources":resources()})
            );
        }
    }
    let blobs_elapsed = start.elapsed();
    let config =
        br#"{"architecture":"amd64","os":"linux","rootfs":{"type":"layers","diff_ids":[]}}"#;
    let mut oci_latency = Vec::with_capacity(20_000);
    let start = Instant::now();
    for i in 0..20_000 {
        let now = Instant::now();
        let config_digest = import(&mut store, config)?;
        let manifest = serde_json::to_vec(
            &serde_json::json!({"schemaVersion":2,"mediaType":"application/vnd.oci.image.manifest.v1+json","config":{"mediaType":"application/vnd.oci.image.config.v1+json","digest":config_digest,"size":config.len()},"layers":[]}),
        )?;
        let digest = import(&mut store, &manifest)?;
        store.admit_oci(
            &digest,
            &Platform {
                os: "linux".into(),
                architecture: "amd64".into(),
                variant: None,
            },
        )?;
        store.pin("oci-churn", &digest)?;
        store.unpin("oci-churn")?;
        ensure!(store.gc(32)? == 2, "OCI churn collection failed");
        oci_latency.push(now.elapsed().as_micros());
        if i % 2000 == 0 {
            println!(
                "{}",
                serde_json::json!({"oci_iteration":i,"resources":resources()})
            );
        }
    }
    println!(
        "{}",
        serde_json::json!({"blob_operations":100000,"oci_lifecycles":20000,"blob_elapsed_seconds":blobs_elapsed.as_secs_f64(),"oci_elapsed_seconds":start.elapsed().as_secs_f64(),"blob_latency":distribution(latency),"oci_latency":distribution(oci_latency),"baseline":baseline,"final_resources":resources(),"final_state":store.status()?})
    );
    Ok(())
}
