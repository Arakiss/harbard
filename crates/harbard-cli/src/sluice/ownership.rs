//! Hold the actual target checkout's writer domain while its files move.

use super::Repository;
use crate::context::Context;
use harbard_core::{
    LeaseClaim, LeaseMetadata, LeaseResource, LeaseStore, Store, SystemLeaseClock, SystemPidChecker,
};
use std::path::Path;

pub(super) fn with_writer<T>(
    repository: &Repository,
    store: &Store,
    ctx: &Context,
    checkout: &Path,
    operation: impl FnOnce() -> Result<T, String>,
) -> Result<T, String> {
    let resource = LeaseResource::writer(
        &ctx.host,
        repository.common_dir.display().to_string(),
        checkout.display().to_string(),
    )
    .map_err(|error| error.to_string())?;
    let metadata = LeaseMetadata {
        session: ctx.session.clone(),
        harness: ctx.harness.clone(),
        // A temporary claim belongs to this invocation, not the long-lived
        // harness. A crash must not strand a reservation behind its supervisor.
        pid: Some(std::process::id()),
        parent: ctx.parent.clone(),
        repository_id: Some(repository.common_dir.display().to_string()),
        branch: None,
        head_sha: None,
    };
    let leases = LeaseStore::at(store.root());
    let acquired = match leases
        .claim(
            &resource,
            &metadata,
            900,
            SystemLeaseClock,
            &SystemPidChecker,
        )
        .map_err(|error| format!("cannot reserve target checkout: {error}"))?
    {
        LeaseClaim::Acquired(record) => Some(record.claim_id),
        // This session or its lineage already owned the checkout. Integration
        // must not release that pre-existing reservation when it finishes.
        LeaseClaim::Renewed(_) => None,
        LeaseClaim::Conflict(holder) => {
            return Err(format!(
                "target checkout is reserved by session {}; not moving its branch or files",
                holder.session
            ));
        }
    };
    let result = operation();
    if let Some(id) = acquired
        && let Err(error) = leases.release_owned(
            &resource,
            &id,
            &metadata,
            SystemLeaseClock,
            &SystemPidChecker,
        )
    {
        // The landing result is already a fact. Report cleanup separately;
        // this process's PID bounds the leftover temporary reservation.
        eprintln!("harbard: could not release temporary target reservation: {error}");
    }
    result
}
