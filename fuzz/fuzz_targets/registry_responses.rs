#![no_main]
//! Exercise the registry library's actual manifest and error response decoders.
//! HTTP/TLS/redirect/authentication policy requires separate live-server tests.
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    if bytes.len() > 65_536 {
        return;
    }
    if let Ok(manifest) = serde_json::from_slice::<oci_client::manifest::OciManifest>(bytes) {
        let encoded = serde_json::to_vec(&manifest).unwrap();
        let reparsed: oci_client::manifest::OciManifest = serde_json::from_slice(&encoded).unwrap();
        assert_eq!(manifest.content_type(), reparsed.content_type());
        assert_eq!(
            serde_json::to_value(manifest).unwrap(),
            serde_json::to_value(reparsed).unwrap()
        );
    }
    if let Ok(envelope) = serde_json::from_slice::<oci_client::errors::OciEnvelope>(bytes) {
        let encoded = serde_json::to_vec(&envelope).unwrap();
        let reparsed: oci_client::errors::OciEnvelope = serde_json::from_slice(&encoded).unwrap();
        assert_eq!(
            serde_json::to_value(envelope).unwrap(),
            serde_json::to_value(reparsed).unwrap()
        );
    }
});
