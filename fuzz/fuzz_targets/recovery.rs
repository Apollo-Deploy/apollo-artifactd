#![no_main]
use apollo_artifactd::{
    Limits, Store,
    state::{Imported, State},
};
use libfuzzer_sys::fuzz_target;
use std::{io::Read, os::unix::fs::PermissionsExt};

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
    let temp_sentinel = root.path().join("temp/foreign");
    std::fs::write(&temp_sentinel, b"unowned stage").unwrap();

    let temp = format!("{IMPORT_ID}.part");
    // An envelope varies persisted metadata independently of staged content.
    // Raw inputs retain the original incomplete-import coverage.
    let envelope = serde_json::from_slice::<serde_json::Value>(bytes).ok();
    let data = envelope
        .as_ref()
        .and_then(|value| value.get("data"))
        .and_then(|value| value.as_str())
        .map_or(bytes, str::as_bytes);
    let record = envelope
        .as_ref()
        .and_then(|value| value.get("record"))
        .cloned()
        .unwrap_or_else(|| {
            serde_json::to_value(Imported {
                temp: temp.clone(),
                digest: None,
                size: data.len() as u64,
            })
            .unwrap()
        });
    let staged = root.path().join("temp").join(&temp);
    std::fs::write(&staged, data).unwrap();
    for path in [&sentinel, &temp_sentinel, &staged] {
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).unwrap();
    }
    if envelope
        .as_ref()
        .and_then(|value| value.get("readonly"))
        .and_then(|value| value.as_bool())
        .unwrap_or(false)
    {
        std::fs::set_permissions(&staged, std::fs::Permissions::from_mode(0o400)).unwrap();
    }
    let dir =
        cap_std::fs::Dir::open_ambient_dir(root.path(), cap_std::ambient_authority()).unwrap();
    let db = State::open(&dir).unwrap();
    db.put("imports", IMPORT_ID, &record).unwrap();
    drop(db);

    match Store::open(root.path(), Limits::default()) {
        Ok(mut store) => {
            assert!(store.doctor().is_ok());
            if let Ok(Imported {
                digest: Some(digest),
                ..
            }) = serde_json::from_value(record.clone())
                && let Ok(mut blob) = store.open_blob(&digest)
            {
                let mut recovered = Vec::new();
                blob.read_to_end(&mut recovered).unwrap();
                assert_eq!(recovered, data);
            }
        }
        Err(_) => {
            // One record is injected: a rejected decode or ownership claim
            // cannot discard the evidence required for operator review.
            let db = State::open(&dir).unwrap();
            assert_eq!(
                db.get::<serde_json::Value>("imports", IMPORT_ID).unwrap(),
                Some(record)
            );
        }
    }
    assert_eq!(std::fs::read(sentinel).unwrap(), b"untouchable");
    assert_eq!(std::fs::read(temp_sentinel).unwrap(), b"unowned stage");
});
