#![no_main]
use apollo_artifactd::{
    Limits, Store,
    state::{Imported, State},
};
use libfuzzer_sys::fuzz_target;
use std::os::unix::fs::PermissionsExt;

const IMPORT_ID: &str = "00000000-0000-4000-8000-000000000000";

fuzz_target!(|bytes: &[u8]| {
    if bytes.len() > 1024 {
        return;
    }
    let root = tempfile::tempdir().unwrap();
    std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    drop(Store::open(root.path(), Limits::default()).unwrap());
    let sentinel = root.path().join("foreign");
    std::fs::write(&sentinel, b"untouchable").unwrap();

    let temp = format!("{IMPORT_ID}.part");
    std::fs::write(root.path().join("temp").join(&temp), bytes).unwrap();
    let dir =
        cap_std::fs::Dir::open_ambient_dir(root.path(), cap_std::ambient_authority()).unwrap();
    let db = State::open(&dir).unwrap();
    db.put(
        "imports",
        IMPORT_ID,
        &Imported {
            temp,
            digest: None,
            size: bytes.len() as u64,
        },
    )
    .unwrap();
    drop(db);

    let _ = Store::open(root.path(), Limits::default());
    assert_eq!(std::fs::read(sentinel).unwrap(), b"untouchable");
});
