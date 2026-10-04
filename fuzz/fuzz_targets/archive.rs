#![no_main]
use apollo_artifactd::{Limits, Store};
use artifactd_protocol::Platform;
use libfuzzer_sys::fuzz_target;
use std::{io::Cursor, os::unix::fs::PermissionsExt};
fuzz_target!(|bytes: &[u8]| {
    if bytes.len() > 65536 { return; }
    let root = tempfile::tempdir().unwrap();
    std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let limits = Limits { max_blob: 65536, max_store: 262144, max_output: 65536,
        max_entry: 65536, max_entries: 64, ..Limits::default() };
    let mut store = Store::open(root.path(), limits).unwrap();
    let p = Platform { os: "linux".into(), architecture: "amd64".into(), variant: None };
    if let Ok(facts) = store.import_oci_archive(Cursor::new(bytes), &p) {
        let manifest = serde_json::from_value(facts["manifest_digest"].clone()).unwrap();
        let _ = store.prepare(&manifest, &p);
    }
    drop(store);
    if let Ok(store) = Store::open(root.path(), limits) {
        assert!(store.doctor().is_ok());
    }
});
