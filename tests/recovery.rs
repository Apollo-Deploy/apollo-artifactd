mod support;
use apollo_artifactd::{Limits, Store};
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
    let db = rusqlite::Connection::open(root.path().join("state.sqlite")).unwrap();
    db.execute(
        "INSERT INTO imports(id,temp,digest,size) VALUES (?1,?2,?3,?4)",
        rusqlite::params![id, temp, digest.as_str(), data.len() as i64],
    )
    .unwrap();
    drop(db);
    let mut store = Store::open(root.path(), Limits::default()).unwrap();
    assert!(store.open_blob(&digest).is_ok());
    assert!(!path.exists());
    assert_eq!(store.reconcile(10).unwrap(), 0);
    drop(store);
    let db = rusqlite::Connection::open(root.path().join("state.sqlite")).unwrap();
    db.execute(
        "INSERT INTO gc(digest,size) VALUES (?1,?2)",
        rusqlite::params![digest.as_str(), data.len() as i64],
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
    let db = rusqlite::Connection::open(root.path().join("state.sqlite")).unwrap();
    db.execute(
        "UPDATE prepared SET phase='intent' WHERE id=?1",
        [prepared.as_str()],
    )
    .unwrap();
    drop(db);
    let mut store = Store::open(root.path(), Limits::default()).unwrap();
    assert_eq!(
        store.prepare(&manifest, &platform("amd64")).unwrap(),
        prepared
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
    store
        .import_blob(&mut PausedInput { first: true }, &digest(b"complete"), 8)
        .unwrap();
}

#[test]
fn sigkill_during_import_never_resolves_partial_content() {
    let root = root();
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "crash_import_worker", "--nocapture"])
        .env("ARTIFACTD_TEST_CRASH_STORE", root.path())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
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
