#![cfg(target_os = "linux")]

#[path = "common/peer.rs"]
mod peer;

use artifactd_protocol::{
    Action, ArtifactDigest, LeaseId, OperationId, PeerIdentity, PinId, Request, VERSION,
    client::Client,
};
use peer::{SERVICE_GID, SERVICE_UID, allocate_operation, response, root_harness};
use rustix::process::geteuid;
use std::{
    io::{Read, Seek, SeekFrom, Write},
    os::unix::fs::PermissionsExt,
    process::{Child, Command, Stdio},
};

const PRODUCER_UID: u32 = 65_533;
const PRODUCER_GID: u32 = 65_532;
const CONSUMER_UID: u32 = 65_531;
const CONSUMER_GID: u32 = 65_530;
const PAYLOAD: &[u8] = b"delegated lease survives daemon restart";

#[test]
#[ignore = "child-process helper; invoked by the root-only delegated lease integration case"]
fn holds_claimed_blob_fd_until_release() {
    let socket = std::env::var_os("ARTIFACTD_LEASE_SOCKET").unwrap();
    let ready = std::env::var_os("ARTIFACTD_LEASE_READY").unwrap();
    let uid = std::env::var("ARTIFACTD_PEER_UID")
        .unwrap()
        .parse()
        .unwrap();
    let gid = std::env::var("ARTIFACTD_PEER_GID")
        .unwrap()
        .parse()
        .unwrap();
    let lease: LeaseId = std::env::var("ARTIFACTD_LEASE_ID")
        .unwrap()
        .try_into()
        .unwrap();
    let digest: ArtifactDigest = std::env::var("ARTIFACTD_LEASE_DIGEST")
        .unwrap()
        .parse()
        .unwrap();
    let release_operation: OperationId = std::env::var("ARTIFACTD_LEASE_RELEASE_OPERATION")
        .unwrap()
        .try_into()
        .unwrap();
    peer::with_peer_credentials(uid, gid, move || {
        let client = Client::new(socket, SERVICE_UID);
        let request = Request {
            version: VERSION,
            operation_id: "consumer-open".to_owned().try_into().unwrap(),
            action: Action::OpenBlob {
                digest,
                lease: lease.clone(),
            },
        };
        let (response, fd) = client.call(&request, None).unwrap();
        assert!(response.result.is_ok());
        let mut blob = std::fs::File::from(fd.unwrap());
        let mut bytes = Vec::new();
        blob.read_to_end(&mut bytes).unwrap();
        assert_eq!(bytes, PAYLOAD);
        std::fs::write(ready, b"ready").unwrap();

        let mut release_signal = [0];
        std::io::stdin()
            .read_exact(&mut release_signal)
            .expect("parent signals lease release after daemon restart and GC");
        blob.seek(SeekFrom::Start(0)).unwrap();
        bytes.clear();
        blob.read_to_end(&mut bytes).unwrap();
        assert_eq!(bytes, PAYLOAD);

        let release = Request {
            version: VERSION,
            operation_id: release_operation,
            action: Action::LeaseRelease { id: lease },
        };
        let (response, fd) = client.call(&release, None).unwrap();
        assert!(response.result.is_ok());
        assert!(fd.is_none());
    });
}

struct Consumer(Option<Child>);

impl Consumer {
    fn child(&mut self) -> &mut Child {
        self.0.as_mut().unwrap()
    }

    fn wait(mut self) -> std::process::ExitStatus {
        self.0.take().unwrap().wait().unwrap()
    }
}

impl Drop for Consumer {
    fn drop(&mut self) {
        if let Some(mut child) = self.0.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

fn result(output: std::process::Output) -> serde_json::Value {
    response(&output).result.unwrap()
}

fn import_payload(harness: &peer::Harness, input: &std::path::Path) -> ArtifactDigest {
    std::fs::write(input, PAYLOAD).unwrap();
    std::fs::set_permissions(input, std::fs::Permissions::from_mode(0o600)).unwrap();
    peer::chown(input, PRODUCER_UID, PRODUCER_GID);
    let imported = result(harness.run_cli(
        PRODUCER_UID,
        PRODUCER_GID,
        &Action::ImportBlob {
            digest: None,
            size: PAYLOAD.len() as u64,
        },
        None,
        Some(input),
    ));
    serde_json::from_value(imported["artifact_digest"].clone()).unwrap()
}

fn create_delegated_lease(
    harness: &peer::Harness,
    digest: &ArtifactDigest,
) -> (String, Action, LeaseId) {
    let operation = allocate_operation(harness, PRODUCER_UID, PRODUCER_GID);
    let action = Action::LeaseCreate {
        digest: digest.clone(),
        grantee: Some(PeerIdentity {
            uid: CONSUMER_UID,
            gid: CONSUMER_GID,
        }),
    };
    let facts =
        result(harness.run_cli(PRODUCER_UID, PRODUCER_GID, &action, Some(&operation), None));
    let lease: LeaseId = facts["lease_id"]
        .as_str()
        .unwrap()
        .to_owned()
        .try_into()
        .unwrap();
    assert_eq!(
        lease.as_str(),
        LeaseId::from_operation(&operation.clone().try_into().unwrap())
            .unwrap()
            .as_str()
    );
    (operation, action, lease)
}

fn admin_gc(harness: &peer::Harness) -> u64 {
    result(harness.run_cli(
        SERVICE_UID,
        SERVICE_GID,
        &Action::Gc { max_entries: 4096 },
        None,
        None,
    ))["collected"]
        .as_u64()
        .unwrap()
}

fn assert_denied(output: &std::process::Output, expected: &str) {
    assert!(!output.status.success(), "operation unexpectedly succeeded");
    assert_eq!(peer::error(output), expected);
}

fn assert_role_retry_and_cleanup(
    harness: &peer::Harness,
    uid: u32,
    gid: u32,
    forbidden: Action,
    valid: Action,
    digest: &ArtifactDigest,
) {
    let operation = allocate_operation(harness, uid, gid);
    let denied = harness.run_cli(uid, gid, &forbidden, Some(&operation), None);
    assert_denied(&denied, "peer role is not authorized for this operation");
    let created = result(harness.run_cli(uid, gid, &valid, Some(&operation), None));
    let lease: LeaseId = created["lease_id"]
        .as_str()
        .unwrap()
        .to_owned()
        .try_into()
        .unwrap();
    let release_operation = allocate_operation(harness, uid, gid);
    result(harness.run_cli(
        uid,
        gid,
        &Action::LeaseRelease { id: lease },
        Some(&release_operation),
        None,
    ));
    assert_eq!(
        result(harness.run_cli(
            SERVICE_UID,
            SERVICE_GID,
            &Action::Verify {
                digest: digest.clone(),
            },
            None,
            None,
        ))["artifact_digest"],
        digest.as_str()
    );
}

fn assert_consumer_grantee_retry(harness: &peer::Harness, digest: &ArtifactDigest) {
    let operation = allocate_operation(harness, CONSUMER_UID, CONSUMER_GID);
    let foreign_grantee = Action::LeaseCreate {
        digest: digest.clone(),
        grantee: Some(PeerIdentity {
            uid: PRODUCER_UID,
            gid: PRODUCER_GID,
        }),
    };
    let denied = harness.run_cli(
        CONSUMER_UID,
        CONSUMER_GID,
        &foreign_grantee,
        Some(&operation),
        None,
    );
    assert_denied(&denied, "consumer may only create a lease for itself");
    let self_lease = result(harness.run_cli(
        CONSUMER_UID,
        CONSUMER_GID,
        &Action::LeaseCreate {
            digest: digest.clone(),
            grantee: None,
        },
        Some(&operation),
        None,
    ));
    let lease: LeaseId = self_lease["lease_id"]
        .as_str()
        .unwrap()
        .to_owned()
        .try_into()
        .unwrap();
    let release_operation = allocate_operation(harness, CONSUMER_UID, CONSUMER_GID);
    result(harness.run_cli(
        CONSUMER_UID,
        CONSUMER_GID,
        &Action::LeaseRelease { id: lease },
        Some(&release_operation),
        None,
    ));
}

#[test]
#[ignore = "requires explicit root execution for isolated numeric UID/GID peers"]
fn delegated_claim_survives_restart_and_release_replay_cannot_rebind() {
    assert_eq!(geteuid().as_raw(), 0, "run this ignored case as root");
    let harness = root_harness(&[
        (PRODUCER_UID, PRODUCER_GID, "producer"),
        (CONSUMER_UID, CONSUMER_GID, "consumer"),
    ]);
    let daemon = harness.spawn_daemon();
    harness.wait_ready();
    let input = harness.root.path().join("input");
    let digest = import_payload(&harness, &input);

    let producer_pin = PinId::try_from("role-check-pin".to_owned()).unwrap();
    assert_role_retry_and_cleanup(
        &harness,
        PRODUCER_UID,
        PRODUCER_GID,
        Action::Gc { max_entries: 1 },
        Action::LeaseCreate {
            digest: digest.clone(),
            grantee: Some(PeerIdentity {
                uid: CONSUMER_UID,
                gid: CONSUMER_GID,
            }),
        },
        &digest,
    );
    assert_role_retry_and_cleanup(
        &harness,
        CONSUMER_UID,
        CONSUMER_GID,
        Action::Pin {
            id: producer_pin,
            digest: digest.clone(),
        },
        Action::LeaseCreate {
            digest: digest.clone(),
            grantee: None,
        },
        &digest,
    );
    assert_consumer_grantee_retry(&harness, &digest);

    let (create_operation, create_action, lease) = create_delegated_lease(&harness, &digest);
    let release_operation = allocate_operation(&harness, CONSUMER_UID, CONSUMER_GID);
    let release_action = Action::LeaseRelease { id: lease.clone() };

    let consumer_dir = harness.root.path().join("consumer");
    std::fs::create_dir(&consumer_dir).unwrap();
    std::fs::set_permissions(&consumer_dir, std::fs::Permissions::from_mode(0o700)).unwrap();
    peer::chown(&consumer_dir, CONSUMER_UID, CONSUMER_GID);
    let ready = consumer_dir.join("ready");
    let mut consumer = Command::new(&harness.helper_binary);
    consumer
        .args([
            "--exact",
            "holds_claimed_blob_fd_until_release",
            "--ignored",
            "--nocapture",
        ])
        .env("ARTIFACTD_PEER_UID", CONSUMER_UID.to_string())
        .env("ARTIFACTD_PEER_GID", CONSUMER_GID.to_string())
        .env("ARTIFACTD_LEASE_SOCKET", &harness.socket)
        .env("ARTIFACTD_LEASE_READY", &ready)
        .env("ARTIFACTD_LEASE_ID", lease.as_str())
        .env("ARTIFACTD_LEASE_DIGEST", digest.as_str())
        .env("ARTIFACTD_LEASE_RELEASE_OPERATION", &release_operation)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let mut consumer = Consumer(Some(consumer.spawn().unwrap()));
    for _ in 0..500 {
        if ready.exists() {
            break;
        }
        assert!(
            consumer.child().try_wait().unwrap().is_none(),
            "consumer helper exited"
        );
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert!(ready.exists(), "consumer did not claim its lease");

    let producer_release = harness.run_cli(PRODUCER_UID, PRODUCER_GID, &release_action, None, None);
    assert_denied(&producer_release, "claimed lease release unauthorized");
    drop(daemon);

    let daemon = harness.spawn_daemon();
    harness.wait_ready();
    assert_eq!(admin_gc(&harness), 0, "claimed lease must protect its blob");
    let verify = result(harness.run_cli(
        SERVICE_UID,
        SERVICE_GID,
        &Action::Verify {
            digest: digest.clone(),
        },
        None,
        None,
    ));
    assert_eq!(verify["artifact_digest"], digest.as_str());

    consumer
        .child()
        .stdin
        .as_mut()
        .unwrap()
        .write_all(b"release")
        .unwrap();
    drop(consumer.child().stdin.take());
    assert!(consumer.wait().success(), "consumer release failed");

    // Replaying the create token returns its receipt but cannot restore a released lease.
    let replay = result(harness.run_cli(
        PRODUCER_UID,
        PRODUCER_GID,
        &create_action,
        Some(&create_operation),
        None,
    ));
    assert_eq!(replay["lease_id"], lease.as_str());
    assert!(
        admin_gc(&harness) > 0,
        "released lease remained protective after create replay"
    );
    let missing = harness.run_cli(
        SERVICE_UID,
        SERVICE_GID,
        &Action::Verify {
            digest: digest.clone(),
        },
        None,
        None,
    );
    assert!(!missing.status.success());

    let digest = import_payload(&harness, &input);
    let (_, _, fresh_lease) = create_delegated_lease(&harness, &digest);
    assert_ne!(
        fresh_lease, lease,
        "fresh create token must mint a distinct lease ID"
    );
    let release_replay = result(harness.run_cli(
        CONSUMER_UID,
        CONSUMER_GID,
        &release_action,
        Some(&release_operation),
        None,
    ));
    assert_eq!(release_replay["lease_id"], lease.as_str());
    assert_eq!(
        admin_gc(&harness),
        0,
        "old release replay removed fresh lease protection"
    );

    let fresh_release_operation = allocate_operation(&harness, CONSUMER_UID, CONSUMER_GID);
    let fresh_release = Action::LeaseRelease { id: fresh_lease };
    result(harness.run_cli(
        CONSUMER_UID,
        CONSUMER_GID,
        &fresh_release,
        Some(&fresh_release_operation),
        None,
    ));
    assert!(admin_gc(&harness) > 0);
    drop(daemon);
}
