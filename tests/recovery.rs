mod support;
use apollo_artifactd::{
    Limits, Store,
    state::{Garbage, Imported, Prepared, State},
};
use std::{
    io::{Cursor, Read},
    os::unix::fs::PermissionsExt,
    process::{Command, Stdio},
    time::{Duration, Instant},
};
use support::*;

fn root() -> tempfile::TempDir {
    let root = tempfile::tempdir().unwrap();
    std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    root
}

#[test]
fn publication_and_gc_intents_reconcile_idempotently() {
    let root = root();
    drop(Store::open(root.path(), Limits::default()).unwrap());
    let data = b"published before database completion";
    let digest = digest(data);
    let id = uuid::Uuid::new_v4().to_string();
    let temp = format!("{id}.part");
    let path = root.path().join("temp").join(&temp);
    std::fs::write(&path, data).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o400)).unwrap();
    std::fs::hard_link(&path, root.path().join("blobs").join(digest.hex())).unwrap();
    let dir =
        cap_std::fs::Dir::open_ambient_dir(root.path(), cap_std::ambient_authority()).unwrap();
    let db = State::open(&dir).unwrap();
    db.put(
        "imports",
        &id,
        &Imported {
            temp: temp.clone(),
            digest: Some(digest.clone()),
            size: data.len() as u64,
        },
    )
    .unwrap();
    drop(db);
    let mut store = Store::open(root.path(), Limits::default()).unwrap();
    assert!(store.open_blob(&digest).is_ok());
    assert!(!path.exists());
    assert_eq!(store.reconcile(10).unwrap(), 0);
    drop(store);
    let db = State::open(&dir).unwrap();
    db.put(
        "gc",
        digest.as_str(),
        &Garbage {
            size: data.len() as u64,
        },
    )
    .unwrap();
    std::fs::remove_file(root.path().join("blobs").join(digest.hex())).unwrap();
    drop(db);
    let mut store = Store::open(root.path(), Limits::default()).unwrap();
    assert!(store.open_blob(&digest).is_err());
    assert_eq!(store.gc(10).unwrap(), 0);
}

#[test]
fn prepared_publication_reconciles_before_reuse() {
    let root = root();
    let mut store = Store::open(root.path(), Limits::default()).unwrap();
    let manifest = image(
        &mut store,
        "amd64",
        &[layer(&[("file", b"payload")])],
        false,
    );
    let prepared = store.prepare(&manifest, &platform("amd64")).unwrap();
    drop(store);
    let dir =
        cap_std::fs::Dir::open_ambient_dir(root.path(), cap_std::ambient_authority()).unwrap();
    let db = State::open(&dir).unwrap();
    let mut record = db
        .get::<Prepared>("prepared", prepared.as_str())
        .unwrap()
        .unwrap();
    record.phase = "intent".to_owned();
    db.put("prepared", prepared.as_str(), &record).unwrap();
    drop(db);
    let mut store = Store::open(root.path(), Limits::default()).unwrap();
    assert_eq!(
        store.prepare(&manifest, &platform("amd64")).unwrap(),
        prepared
    );
}

#[test]
fn prepared_gc_intent_reconciles_with_or_without_tree_effect() {
    for tree_effect_completed in [false, true] {
        let root = root();
        let mut store = Store::open(root.path(), Limits::default()).unwrap();
        let manifest = image(&mut store, "amd64", &[], false);
        let prepared = store.prepare(&manifest, &platform("amd64")).unwrap();
        drop(store);

        let dir =
            cap_std::fs::Dir::open_ambient_dir(root.path(), cap_std::ambient_authority()).unwrap();
        let db = State::open(&dir).unwrap();
        let mut record = db
            .get::<Prepared>("prepared", prepared.as_str())
            .unwrap()
            .unwrap();
        record.phase = "gc_intent".to_owned();
        db.put("prepared", prepared.as_str(), &record).unwrap();
        if tree_effect_completed {
            std::fs::remove_dir_all(root.path().join("prepared").join(prepared.as_str())).unwrap();
        }
        drop(db);

        let reopened = Store::open(root.path(), Limits::default()).unwrap();
        assert!(reopened.open_blob(&manifest).is_ok());
        assert!(
            !root
                .path()
                .join("prepared")
                .join(prepared.as_str())
                .exists()
        );
        drop(reopened);
        let db = State::open(&dir).unwrap();
        assert!(
            db.get::<Prepared>("prepared", prepared.as_str())
                .unwrap()
                .is_none()
        );
        drop(db);
        let mut reopened = Store::open(root.path(), Limits::default()).unwrap();
        assert_eq!(reopened.reconcile(10).unwrap(), 0);
    }
}

#[test]
fn prepared_recovery_cursor_reaches_late_intent_after_many_complete_records() {
    let root = root();
    drop(Store::open(root.path(), Limits::default()).unwrap());
    let dir =
        cap_std::fs::Dir::open_ambient_dir(root.path(), cap_std::ambient_authority()).unwrap();
    let db = State::open(&dir).unwrap();
    let manifest = digest(b"recovery-manifest");
    std::fs::write(
        root.path().join("blobs").join(manifest.hex()),
        b"recovery-manifest",
    )
    .unwrap();
    std::fs::set_permissions(
        root.path().join("blobs").join(manifest.hex()),
        std::fs::Permissions::from_mode(0o400),
    )
    .unwrap();
    for n in 0..4096u32 {
        let id = format!("{n:064x}");
        std::fs::create_dir(root.path().join("prepared").join(&id)).unwrap();
        std::fs::set_permissions(
            root.path().join("prepared").join(&id),
            std::fs::Permissions::from_mode(0o500),
        )
        .unwrap();
    }
    db.transaction(|tx| {
        tx.put(
            "blobs",
            manifest.as_str(),
            &apollo_artifactd::state::Blob { size: 17 },
        )?;
        for n in 0..4096u32 {
            let id = format!("{n:064x}");
            tx.put(
                "prepared",
                &id,
                &Prepared {
                    manifest: manifest.clone(),
                    platform: platform("amd64"),
                    phase: "complete".into(),
                    tree_digest: Some(digest(b"").hex().to_string()),
                    staging: format!("{}.staging", uuid::Uuid::new_v4()),
                    size: 0,
                },
            )?;
        }
        tx.put(
            "prepared",
            &format!("{:064x}", 4096u32),
            &Prepared {
                manifest,
                platform: platform("amd64"),
                phase: "intent".into(),
                tree_digest: None,
                staging: format!("{}.staging", uuid::Uuid::new_v4()),
                size: 0,
            },
        )
    })
    .unwrap();
    drop(db);

    let mut store = Store::open(root.path(), Limits::default()).unwrap();
    store.reconcile(4096).unwrap();
    drop(store);
    let db = State::open(&dir).unwrap();
    assert_eq!(db.count("prepared").unwrap(), 4096);
    assert!(
        db.get::<Prepared>("prepared", &format!("{:064x}", 4096u32))
            .unwrap()
            .is_none()
    );
}

struct PausedInput {
    first: bool,
}
impl Read for PausedInput {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if self.first {
            self.first = false;
            return Cursor::new(b"partial").read(buf);
        }
        loop {
            std::thread::sleep(Duration::from_secs(1));
        }
    }
}

#[test]
fn crash_import_worker() {
    let Ok(path) = std::env::var("ARTIFACTD_TEST_CRASH_STORE") else {
        return;
    };
    let mut store = Store::open(std::path::Path::new(&path), Limits::default()).unwrap();
    if std::env::var_os("ARTIFACTD_TEST_CALCULATED_IMPORT").is_some() {
        store
            .import_calculated(&mut PausedInput { first: true }, 8)
            .unwrap();
    } else {
        store
            .import_blob(&mut PausedInput { first: true }, &digest(b"complete"), 8)
            .unwrap();
    }
}

#[test]
fn sigkill_during_import_never_resolves_partial_content() {
    kill_import(false);
}

#[test]
fn sigkill_during_calculated_import_discards_unresolved_intent() {
    kill_import(true);
}

fn kill_import(calculated: bool) {
    let root = root();
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .args(["--exact", "crash_import_worker", "--nocapture"])
        .env("ARTIFACTD_TEST_CRASH_STORE", root.path())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    if calculated {
        command.env("ARTIFACTD_TEST_CALCULATED_IMPORT", "1");
    }
    let mut child = command.spawn().unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    let partial = loop {
        let files = std::fs::read_dir(root.path().join("temp"));
        if let Ok(files) = files
            && let Some(file) = files
                .filter_map(Result::ok)
                .find(|f| f.metadata().is_ok_and(|m| m.len() > 0))
        {
            break file.path();
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("crash worker did not reach import effect");
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    child.kill().unwrap();
    assert!(!child.wait().unwrap().success());
    let store = Store::open(root.path(), Limits::default()).unwrap();
    assert!(!partial.exists());
    assert!(store.open_blob(&digest(b"complete")).is_err());
    assert_eq!(store.status().unwrap()["blobs"], 0);
}
