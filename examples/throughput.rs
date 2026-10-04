//! Streaming throughput probe; fixtures are generated outside measured imports.
use anyhow::{Result, ensure};
use apollo_artifactd::{Limits, Store};
use artifactd_protocol::{ArtifactDigest, Platform};
use sha2::{Digest, Sha256};
use std::{
    io::{Read, Seek, SeekFrom, Write},
    path::PathBuf,
    time::Instant,
};

const SIZE: u64 = 256 << 20;
struct Repeated(u64);
impl Read for Repeated {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        let n = buffer.len().min(self.0 as usize);
        buffer[..n].fill(0x61);
        self.0 -= n as u64;
        Ok(n)
    }
}
fn digest(reader: &mut impl Read) -> Result<ArtifactDigest> {
    let mut hash = Sha256::new();
    let mut buffer = [0; 65536];
    loop {
        let n = reader.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        hash.update(&buffer[..n]);
    }
    Ok(format!("sha256:{}", hex::encode(hash.finalize())).parse()?)
}
fn bytes(store: &mut Store, bytes: &[u8]) -> Result<ArtifactDigest> {
    let d = digest(&mut &*bytes)?;
    store.import_blob(&mut &*bytes, &d, bytes.len() as u64)?;
    Ok(d)
}
fn main() -> Result<()> {
    let path: PathBuf = std::env::args()
        .nth(1)
        .ok_or_else(|| anyhow::anyhow!("private store path required"))?
        .into();
    let mut store = Store::open(&path, Limits::default())?;
    let d = digest(&mut Repeated(SIZE))?;
    let now = Instant::now();
    store.import_blob(&mut Repeated(SIZE), &d, SIZE)?;
    let import_seconds = now.elapsed().as_secs_f64();
    let mut layer = tempfile::tempfile()?;
    {
        let mut archive = tar::Builder::new(&mut layer);
        let mut header = tar::Header::new_gnu();
        header.set_size(SIZE);
        header.set_mode(0o644);
        header.set_cksum();
        archive.append_data(&mut header, "payload", Repeated(SIZE))?;
        archive.finish()?;
    }
    layer.flush()?;
    let size = layer.metadata()?.len();
    layer.seek(SeekFrom::Start(0))?;
    let layer_digest = digest(&mut layer)?;
    layer.seek(SeekFrom::Start(0))?;
    store.import_blob(&mut layer, &layer_digest, size)?;
    let config = serde_json::to_vec(
        &serde_json::json!({"architecture":"amd64","os":"linux","rootfs":{"type":"layers","diff_ids":[layer_digest]}}),
    )?;
    let config_digest = bytes(&mut store, &config)?;
    let manifest = serde_json::to_vec(
        &serde_json::json!({"schemaVersion":2,"mediaType":"application/vnd.oci.image.manifest.v1+json","config":{"mediaType":"application/vnd.oci.image.config.v1+json","digest":config_digest,"size":config.len()},"layers":[{"mediaType":"application/vnd.oci.image.layer.v1.tar","digest":layer_digest,"size":size}]}),
    )?;
    let manifest = bytes(&mut store, &manifest)?;
    let platform = Platform {
        os: "linux".into(),
        architecture: "amd64".into(),
        variant: None,
    };
    store.admit_oci(&manifest, &platform)?;
    let now = Instant::now();
    let prepared = store.prepare(&manifest, &platform)?;
    let prepare_seconds = now.elapsed().as_secs_f64();
    store.lease("benchmark", &manifest)?;
    let dir = cap_std::fs::Dir::from_std_file(store.open_prepared(&prepared, "benchmark")?);
    ensure!(
        dir.metadata("payload")?.len() == SIZE,
        "prepared size mismatch"
    );
    println!(
        "{}",
        serde_json::json!({"bytes":SIZE,"import_seconds":import_seconds,"import_mib_per_second":256.0/import_seconds,"prepare_seconds":prepare_seconds,"prepare_mib_per_second":256.0/prepare_seconds,"verification_and_fsync":true,"fixture":"one uncompressed layer"})
    );
    Ok(())
}
