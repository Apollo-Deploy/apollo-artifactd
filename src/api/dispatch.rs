use crate::Store;
use anyhow::{Result, bail, ensure};
use artifactd_protocol::{Action, Request, VERSION};
use rusqlite::OptionalExtension;
use rustix::fd::OwnedFd;
use serde_json::{Value, json};

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
    ensure!(
        fd.is_some() == input_required,
        "unexpected/missing input descriptor"
    );
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
    if matches!(
        request.action,
        Action::Status
            | Action::Doctor
            | Action::Capabilities
            | Action::Inspect { .. }
            | Action::Verify { .. }
            | Action::Resolve { .. }
            | Action::EnsureLocal { .. }
            | Action::OpenBlob { .. }
            | Action::OpenPrepared { .. }
    ) {
        return dispatch(store, &request.action, input);
    }
    let payload = serde_json::to_string(&request.action)?;
    let prior: Option<(String, String, Option<String>)> = store
        .db
        .query_row(
            "SELECT request,phase,result FROM operations WHERE id=?1",
            [request.operation_id.as_str()],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()?;
    let open = matches!(
        request.action,
        Action::OpenBlob { .. } | Action::OpenPrepared { .. }
    );
    if let Some((old, phase, result)) = prior {
        ensure!(old == payload, "operation identity conflict");
        if phase == "complete" && !open {
            let result: Result<Value, String> = serde_json::from_str(
                &result.ok_or_else(|| anyhow::anyhow!("missing operation result"))?,
            )?;
            return result.map(|v| (v, None)).map_err(anyhow::Error::msg);
        }
    } else {
        let pending: u32 = store.db.query_row(
            "SELECT count(*) FROM operations WHERE phase='intent'",
            [],
            |r| r.get(0),
        )?;
        ensure!(pending < 4096, "unfinished operation limit");
        store.db.execute(
            "INSERT INTO operations(id,request,phase) VALUES (?1,?2,'intent')",
            rusqlite::params![request.operation_id.as_str(), payload],
        )?;
    }
    let outcome = dispatch(store, &request.action, input);
    let saved: Result<&Value, String> = outcome.as_ref().map(|(v, _)| v).map_err(|e| e.to_string());
    store.db.execute(
        "UPDATE operations SET phase=?3,result=?2 WHERE id=?1",
        rusqlite::params![
            request.operation_id.as_str(),
            serde_json::to_string(&saved)?,
            if outcome.is_ok() {
                "complete"
            } else {
                "failed"
            }
        ],
    )?;
    // Bound history without pruning unfinished mutation intents. Each effect is
    // independently idempotent; IDs older than this window may be re-executed.
    store.db.execute("DELETE FROM operations WHERE phase IN ('complete','failed') AND seq < (SELECT coalesce(max(seq),0)-4096 FROM operations)", [])?;
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
            store.import_blob(&mut file, digest, *size)?;
            json!({"artifact_digest":digest,"size":size})
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
            crate::oci::facts(&store.resolve(digest, platform)?, platform)
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
            json!({"prepared_artifact_id":store.prepare(digest,platform)?})
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
        Action::Status => store.status()?,
        Action::Doctor => store.doctor()?,
        Action::Capabilities => {
            json!({"version":VERSION,"fd_transport":"SCM_RIGHTS","credentials":"SO_PEERCRED","platforms":["linux/amd64","linux/arm64"],"registry":false,"production_qualified":false})
        }
        Action::Pull { .. } | Action::Push { .. } => {
            bail!("registry operations unavailable: production qualification incomplete")
        }
    };
    Ok((value, fd))
}
