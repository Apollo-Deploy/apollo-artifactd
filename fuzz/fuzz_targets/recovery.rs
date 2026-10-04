#![no_main]
use apollo_artifactd::{Limits, Store};
use libfuzzer_sys::fuzz_target;
use std::os::unix::fs::PermissionsExt;
fuzz_target!(|bytes: &[u8]| {
    if bytes.len() > 1024 { return; }
    let root = tempfile::tempdir().unwrap();
    std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    drop(Store::open(root.path(), Limits::default()).unwrap());
    let sentinel = root.path().join("foreign");
    std::fs::write(&sentinel, b"untouchable").unwrap();
    let db = rusqlite::Connection::open(root.path().join("state.sqlite")).unwrap();
    let text = String::from_utf8_lossy(bytes);
    db.execute("INSERT INTO imports(id,temp,digest,size) VALUES (?1,?1,?1,?2)",
        rusqlite::params![text.as_ref(), bytes.len() as i64]).unwrap();
    drop(db);
    let _ = Store::open(root.path(), Limits::default());
    assert_eq!(std::fs::read(sentinel).unwrap(), b"untouchable");
});
