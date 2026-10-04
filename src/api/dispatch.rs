use crate::Store;
use anyhow::{Result, ensure};
use artifactd_protocol::{Action, Request, VERSION};
use rustix::fd::OwnedFd;
use serde_json::{Value, json};
static REGISTRY: std::sync::LazyLock<Result<crate::registry::Registry>> =
    std::sync::LazyLock::new(crate::registry::Registry::new);

pub(super) fn execute(
    store: &mut Store,
    request: &Request,
    fd: Option<OwnedFd>,
) -> (Result<Value, String>, Option<OwnedFd>) {
    match recorded(store, request, fd) {
        Ok((value, fd)) => (Ok(value), fd),
        Err(e) => (Err(e.to_string()), None),
    }
}
fn recorded(
    store: &mut Store,
    request: &Request,
    fd: Option<OwnedFd>,
) -> Result<(Value, Option<OwnedFd>)> {
    let input_required = matches!(
        request.action,
        Action::ImportBlob { .. } | Action::ImportOciArchive { .. }
    );
    let registry = matches!(request.action, Action::Pull { .. } | Action::Push { .. });
    ensure!(
        registry || fd.is_some() == input_required,
        "unexpected/missing input descriptor"
    );
    // Validate references before serializing operation intent: credentials may
    // only enter through a descriptor, never through a URL or query string.
    match &request.action {
        Action::Pull { reference, .. } => {
            crate::registry::validate_reference(reference, true)?;
        }
        Action::Push { digest, reference } => {
            let target = crate::registry::validate_reference(reference, false)?;
            ensure!(
                target.digest().is_none_or(|v| v == digest.as_str()),
                "push destination digest mismatch"
            );
        }
        _ => {}
    }
    let input = fd.map(std::fs::File::from);
    if let Some(file) = &input {
        let stat = rustix::fs::fstat(file)?;
        ensure!(
            rustix::fs::FileType::from_raw_mode(stat.st_mode) == rustix::fs::FileType::RegularFile
                && stat.st_size >= 0
                && (stat.st_size as u64) <= store.limits.max_store,
            "input descriptor must be a bounded regular file"
        );
    }
    // Observations must reflect current integrity and liveness, including replay.
    if !request.action.is_mutation() {
        return dispatch(store, &request.action, input);
    }
    let payload = serde_json::to_string(&request.action)?;
    if let Some(result) = super::journal::begin(store, &request.operation_id, &payload, registry)? {
        return result.map(|v| (v, None)).map_err(anyhow::Error::msg);
    }
    let outcome = dispatch(store, &request.action, input);
    let saved: Result<&Value, String> = outcome.as_ref().map(|(v, _)| v).map_err(|e| e.to_string());
    super::journal::complete(store, &request.operation_id, &payload, &saved, registry)?;
    outcome
}
fn dispatch(
    store: &mut Store,
    action: &Action,
    input: Option<std::fs::File>,
) -> Result<(Value, Option<OwnedFd>)> {
    let mut fd = None;
    let value = match action {
        Action::ImportBlob { digest, size } => {
            let mut file = input.ok_or_else(|| anyhow::anyhow!("missing blob FD"))?;
            let actual = if let Some(expected) = digest {
                store.import_blob(&mut file, expected, *size)?;
                expected.clone()
            } else {
                store.import_calculated(&mut file, *size)?
            };
            json!({"artifact_digest":actual,"size":size})
        }
        Action::ImportOci { digest, platform } => store.admit_oci(digest, platform)?,
        Action::ImportOciArchive { platform } => store.import_oci_archive(
            input.ok_or_else(|| anyhow::anyhow!("missing archive FD"))?,
            platform,
        )?,
        Action::Inspect { digest } | Action::Verify { digest } | Action::EnsureLocal { digest } => {
            let file = store.open_blob(digest)?;
            store.verify_graph_if_known(digest)?;
            json!({"artifact_digest":digest,"size":file.metadata()?.len(),"verified":true})
        }
        Action::Resolve { digest, platform } => {
            crate::oci::facts(digest, &store.resolve(digest, platform)?, platform)
        }
        Action::Pin { id, digest } => {
            store.pin(id.as_str(), digest)?;
            json!({"pin_id":id})
        }
        Action::Unpin { id } => {
            store.unpin(id.as_str())?;
            json!({"pin_id":id})
        }
        Action::LeaseCreate { id, digest } => {
            store.lease(id.as_str(), digest)?;
            json!({"lease_id":id})
        }
        Action::LeaseRelease { id } => {
            store.release(id.as_str())?;
            json!({"lease_id":id})
        }
        Action::Prepare { digest, platform } => {
            let id = store.prepare(digest, platform)?;
            let prepared = store
                .db
                .get::<crate::state::Prepared>("prepared", id.as_str())?
                .ok_or_else(|| anyhow::anyhow!("missing prepared receipt"))?;
            let tree_digest = prepared
                .tree_digest
                .ok_or_else(|| anyhow::anyhow!("missing prepared digest"))?;
            let prepared_digest: artifactd_protocol::PreparedDigest =
                format!("sha256:{tree_digest}").parse()?;
            json!({"prepared_artifact_id":id,"prepared_digest":prepared_digest,
                "manifest_digest":prepared.manifest,"platform":prepared.platform,"size":prepared.size,
                "format":"artifactd-rootfs-v1"})
        }
        Action::OpenBlob { digest, lease } => {
            ensure!(
                store.leased(lease.as_str(), digest)?,
                "blob requires covering lease"
            );
            fd = Some(store.open_blob(digest)?.into());
            json!({"artifact_digest":digest})
        }
        Action::OpenPrepared { id, lease } => {
            fd = Some(store.open_prepared(id, lease.as_str())?.into());
            json!({"prepared_artifact_id":id})
        }
        Action::Gc { max_entries } => json!({"collected":store.gc(*max_entries)?}),
        Action::Reconcile { max_operations } => {
            json!({"recovered":store.reconcile(*max_operations)?})
        }
        Action::OperationAllocate => {
            let id = super::journal::allocate(store)?;
            json!({"operation_id":id,"retention_window":super::journal::WINDOW})
        }
        Action::Status => store.status()?,
        Action::Doctor => {
            let mut facts = store.doctor()?;
            facts["operation_journal"] = super::journal_recovery::audit(store, false)?;
            facts
        }
        Action::Capabilities => {
            json!({"version":VERSION,"fd_transport":"SCM_RIGHTS","credentials":"SO_PEERCRED","registry_credentials":"PRIVATE_FD","platforms":["linux/amd64","linux/arm64"],"registry":true,"production_qualified":false})
        }
        Action::Pull {
            reference,
            platform,
        } => {
            let registry = REGISTRY
                .as_ref()
                .map_err(|_| anyhow::anyhow!("registry runtime unavailable"))?;
            registry.pull(
                store,
                reference,
                platform,
                crate::registry::Credentials::read(input)?,
            )?
        }
        Action::Push { digest, reference } => {
            let registry = REGISTRY
                .as_ref()
                .map_err(|_| anyhow::anyhow!("registry runtime unavailable"))?;
            registry.push(
                store,
                digest,
                reference,
                crate::registry::Credentials::read(input)?,
            )?
        }
    };
    Ok((value, fd))
}
