#![cfg(target_os = "linux")]

#[path = "common/peer.rs"]
mod peer;

use artifactd_protocol::{Action, ArtifactDigest, LeaseId, PinId};
use peer::{SERVICE_GID, SERVICE_UID, allocate_operation, chown, error, response, root_harness};
use rustix::process::geteuid;
use std::{os::unix::fs::PermissionsExt, process::Output};

const FOREIGN_GID: u32 = SERVICE_GID - 1;

fn assert_foreign_operation(
    output: &Output,
    harness: &peer::Harness,
    operation: &str,
    action: &Action,
) {
    assert!(!output.status.success());
    assert!(error(output).contains("operation owner mismatch"));
    let retry = harness.run_cli(SERVICE_UID, FOREIGN_GID, action, Some(operation), None);
    assert!(!retry.status.success());
    assert!(error(&retry).contains("operation owner mismatch"));
}

#[test]
#[ignore = "requires explicit root execution for isolated numeric UID/GID peers"]
fn kernel_peer_gid_binds_operation_owner_across_restart() {
    assert_eq!(geteuid().as_raw(), 0, "run this ignored case as root");
    let harness = root_harness(&[(SERVICE_UID, FOREIGN_GID, "producer")]);
    let daemon = harness.spawn_daemon();

    let ready = harness.wait_ready();
    assert!(response(&ready).result.is_ok());
    let token = allocate_operation(&harness, SERVICE_UID, SERVICE_GID);
    let action = Action::Unpin {
        id: "kernel-owner".to_owned().try_into().unwrap(),
    };
    let foreign = harness.run_cli(SERVICE_UID, FOREIGN_GID, &action, Some(&token), None);
    assert_foreign_operation(&foreign, &harness, &token, &action);
    let rightful =
        response(&harness.run_cli(SERVICE_UID, SERVICE_GID, &action, Some(&token), None));
    assert!(rightful.result.is_ok());
    drop(daemon);

    let daemon = harness.spawn_daemon();
    harness.wait_ready();
    let foreign = harness.run_cli(SERVICE_UID, FOREIGN_GID, &action, Some(&token), None);
    assert_foreign_operation(&foreign, &harness, &token, &action);
    let replay = response(&harness.run_cli(SERVICE_UID, SERVICE_GID, &action, Some(&token), None));
    assert!(replay.result.is_ok());
    drop(daemon);
}

#[test]
#[ignore = "requires explicit root execution for isolated numeric UID/GID peers"]
fn kernel_peer_gid_binds_persistent_pin_and_lease_owners() {
    assert_eq!(geteuid().as_raw(), 0, "run this ignored case as root");
    let harness = root_harness(&[(SERVICE_UID, FOREIGN_GID, "producer")]);
    let input = harness.root.path().join("input");
    std::fs::write(&input, b"reference-owner").unwrap();
    std::fs::set_permissions(&input, std::fs::Permissions::from_mode(0o644)).unwrap();
    chown(&input, SERVICE_UID, SERVICE_GID);
    let daemon = harness.spawn_daemon();
    harness.wait_ready();
    let imported = response(&harness.run_cli(
        SERVICE_UID,
        SERVICE_GID,
        &Action::ImportBlob {
            digest: None,
            size: b"reference-owner".len() as u64,
        },
        None,
        Some(&input),
    ));
    let digest: ArtifactDigest =
        serde_json::from_value(imported.result.unwrap()["artifact_digest"].clone()).unwrap();
    let pin: PinId = "kernel-pin".to_owned().try_into().unwrap();
    let pin_action = Action::Pin {
        id: pin.clone(),
        digest: digest.clone(),
    };
    let create_operation = allocate_operation(&harness, SERVICE_UID, SERVICE_GID);
    let create_action = Action::LeaseCreate {
        digest: digest.clone(),
        grantee: None,
    };
    let created = response(&harness.run_cli(
        SERVICE_UID,
        SERVICE_GID,
        &create_action,
        Some(&create_operation),
        None,
    ));
    let lease: LeaseId = created.result.unwrap()["lease_id"]
        .as_str()
        .unwrap()
        .to_owned()
        .try_into()
        .unwrap();
    let release_action = Action::LeaseRelease { id: lease.clone() };
    let open_action = Action::OpenBlob {
        digest: digest.clone(),
        lease: lease.clone(),
    };
    let owner = |action: &Action| {
        response(&harness.run_cli(SERVICE_UID, SERVICE_GID, action, None, None))
            .result
            .unwrap()
    };
    owner(&pin_action);
    let assert_refs_rejected = || {
        for action in [
            pin_action.clone(),
            Action::Unpin { id: pin.clone() },
            release_action.clone(),
            open_action.clone(),
        ] {
            let output = harness.run_cli(SERVICE_UID, FOREIGN_GID, &action, None, None);
            assert!(!output.status.success());
            let expected = match action {
                Action::OpenBlob { .. } => "lease grantee mismatch",
                Action::LeaseRelease { .. } => "lease release unauthorized",
                _ => "reference owner mismatch",
            };
            assert_eq!(error(&output), expected);
        }
    };
    assert_refs_rejected();
    drop(daemon);
    let daemon = harness.spawn_daemon();
    harness.wait_ready();
    assert_refs_rejected();
    owner(&Action::Gc { max_entries: 4096 });
    owner(&Action::Verify {
        digest: digest.clone(),
    });
    owner(&pin_action);
    let replayed_create = response(&harness.run_cli(
        SERVICE_UID,
        SERVICE_GID,
        &create_action,
        Some(&create_operation),
        None,
    ));
    let replayed_lease = replayed_create.result.unwrap()["lease_id"]
        .as_str()
        .unwrap()
        .to_owned();
    assert_eq!(replayed_lease, lease.as_str());
    owner(&Action::Unpin { id: pin });
    owner(&release_action);
    owner(&Action::Gc { max_entries: 4096 });

    let verify = harness.run_cli(
        SERVICE_UID,
        SERVICE_GID,
        &Action::Verify { digest },
        None,
        None,
    );
    assert!(!verify.status.success());
    assert!(error(&verify).contains("blob is not admitted"));
    drop(daemon);
}
