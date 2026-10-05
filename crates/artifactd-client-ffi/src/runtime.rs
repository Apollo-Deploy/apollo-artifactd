use super::*;
use oci_spec::image::ImageConfiguration;
use std::io::Read;

/// Parse a descriptor-pinned OCI config through a verified artifactd lease.
/// OCI decoding remains a Rust library concern; no storage or registry logic
/// is implemented in this client crate.
#[unsafe(no_mangle)]
pub extern "C" fn artifactd_runtime_config(
    client: *mut ArtifactdClient,
    operation: *const c_char,
    config_digest: *const c_char,
    lease_id: *const c_char,
    out: *mut *mut ArtifactdFacts,
) -> i32 {
    let result = (|| {
        if out.is_null() {
            return Err(Error::Null);
        }
        let client = unsafe { client.as_ref() }.ok_or(Error::Null)?;
        let fd = actions::open_blob_fd(client, operation, config_digest, lease_id)?;
        let mut file = unsafe { File::from_raw_fd(fd) };
        let mut bytes = Vec::new();
        file.by_ref()
            .take(4 * 1024 * 1024 + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| Error::Call(e.to_string()))?;
        if bytes.len() > 4 * 1024 * 1024 {
            return Err(Error::Call("OCI config exceeds 4 MiB".into()));
        }
        let config: ImageConfiguration =
            serde_json::from_slice(&bytes).map_err(|e| Error::Call(e.to_string()))?;
        let runtime = config.config().clone().unwrap_or_default();
        facts(
            serde_json::json!({
                "user": runtime.user().clone().unwrap_or_default(),
                "working_directory": runtime.working_dir().clone().unwrap_or_default(),
                "environment": runtime.env().clone().unwrap_or_default(),
                "entrypoint": runtime.entrypoint().clone().unwrap_or_default(),
                "command": runtime.cmd().clone().unwrap_or_default(),
                "os": config.os().to_string(),
                "architecture": config.architecture().to_string(),
                "variant": config.variant().clone(),
            }),
            out,
        )
    })();
    result.map_or_else(fail, |_| succeed())
}
