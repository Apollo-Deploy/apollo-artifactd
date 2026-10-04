#![no_main]
use libfuzzer_sys::fuzz_target;
fuzz_target!(|bytes: &[u8]| {
    if bytes.len() > 65536 { return; }
    let _ = serde_json::from_slice::<artifactd_protocol::Request>(bytes);
    let _ = serde_json::from_slice::<artifactd_protocol::Response>(bytes);
    let _ = serde_json::from_slice::<oci_spec::image::ImageManifest>(bytes);
    let _ = serde_json::from_slice::<oci_spec::image::ImageIndex>(bytes);
    let _ = serde_json::from_slice::<oci_spec::image::ImageConfiguration>(bytes);
    if let Ok(s) = std::str::from_utf8(bytes) {
        if let Ok(d) = s.parse::<artifactd_protocol::ArtifactDigest>() {
            assert_eq!(d.as_str(), s);
            assert_eq!(s.len(), 71);
            assert!(s.starts_with("sha256:"));
        }
    }
});
