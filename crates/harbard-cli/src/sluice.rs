//! `deliver` and `integrate`: the queue for the right to move a branch.
//!
//! The Git work lives here rather than in the core because every step is a
//! process boundary, and because the guarantee this exists to keep is about
//! what a real repository ends up pointing at.
//!
//! The guarantee: **the commit that lands is the commit the checks ran on.**
//! Integration builds a candidate by merging the delivered SHA onto the CURRENT
//! tip, runs the checks against that candidate in its own worktree, and then
//! fast-forwards the branch to exactly that candidate — or does nothing at all.
//! No rebase after the checks, no "same changes on a newer base", no second
//! build. If the tip moved while the checks ran, the branch does not move and
//! the attempt receives a failure receipt; the owner may deliver again.

use crate::cli::Common;
use crate::context::Context;
use fs2::FileExt;
use harbard_core::sluice::{self, Delivery, Receipt, Verdict};
use harbard_core::{LeaseResource, LeaseStore, Store, SystemLeaseClock, SystemPidChecker};
use serde_json::json;
use std::path::Path;
use std::process::Command;
use ulid::Ulid;

mod landing;
mod repository;
use landing::land_candidate;
mod ownership;
use repository::Repository;

/// Run a Git command in `repo`, returning trimmed stdout on success.
fn git(repo: &Path, args: &[&str]) -> Result<String, String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .output()
        .map_err(|e| format!("git {}: {e}", args.join(" ")))?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());
    }
    let stdout = String::from_utf8_lossy(&out.stdout);
    Ok(if args.contains(&"-z") {
        stdout.into_owned()
    } else {
        stdout.trim().to_string()
    })
}

fn resolve(repo: &Path, rev: &str) -> Result<String, String> {
    // `^{commit}` makes this refuse a tree or a tag object: a delivery has to
    // name something that can actually be merged.
    git(
        repo,
        &["rev-parse", "--verify", &format!("{rev}^{{commit}}")],
    )
}

/// `harbard deliver` — offer a commit for integration.
#[allow(clippy::too_many_arguments)]
pub fn cmd_deliver(
    store: &Store,
    common: &Common,
    repo: &str,
    sha: &str,
    branch: &str,
    checks: &str,
    prepare: &str,
    now: u64,
) -> i32 {
    let ctx = Context::resolve(common);
    let repository = match Repository::resolve(repo) {
        Ok(r) => r,
        Err(e) => return fail(common, &format!("not a Git repository: {e}")),
    };
    let repo_root = &repository.identity;
    let root = repository.root.as_path();

    // B2/B3: refuse before writing anything. A queued delivery that names a
    // commit nobody can find is a rejection deferred to the worst moment —
    // integration time, when the operator is not watching.
    let delivered = match resolve(root, sha) {
        Ok(s) => s,
        Err(e) => return fail(common, &format!("unknown commit \"{sha}\": {e}")),
    };
    let base = match resolve(root, branch) {
        Ok(s) => s,
        Err(e) => return fail(common, &format!("unknown branch \"{branch}\": {e}")),
    };

    let delivery = Delivery {
        id: Ulid::generate().to_string(),
        repo: repo_root.clone(),
        sha: delivered.clone(),
        branch: branch.to_string(),
        base,
        checks: checks.to_string(),
        prepare: prepare.to_string(),
        session: ctx.session.clone(),
        harness: ctx.harness.clone(),
        created_ms: now,
    };

    if let Err(e) = sluice::enqueue(store.root(), &delivery) {
        return fail(common, &format!("could not queue the delivery: {e}"));
    }

    let queued = repository.pending(store.root()).len();
    if common.json {
        println!(
            "{}",
            json!({
                "status": "queued",
                "delivery": delivery,
                "pending": queued,
            })
        );
    } else {
        println!(
            "queued {} · {} onto {} · {} ahead of it",
            &delivery.id[..8.min(delivery.id.len())],
            &delivered[..7.min(delivered.len())],
            branch,
            queued.saturating_sub(1)
        );
    }
    0
}

/// `harbard integrate` — drain the queue for one repo.
pub fn cmd_integrate(store: &Store, common: &Common, repo: &str, now: u64) -> i32 {
    let ctx = Context::resolve(common);
    let repository = match Repository::resolve(repo) {
        Ok(r) => r,
        Err(e) => return fail(common, &format!("not a Git repository: {e}")),
    };
    let repo_root = &repository.identity;
    // A process lock fences concurrent invocations even from the same session
    // and outlives a service lease TTL while checks are still running.
    let lock_dir = repository.common_dir.join("harbard-sluice");
    let lock = (|| {
        std::fs::create_dir_all(&lock_dir)?;
        let file = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(lock_dir.join("integrator.lock"))?;
        file.try_lock_exclusive()?;
        Ok::<_, std::io::Error>(file)
    })();
    let _lock = match lock {
        Ok(file) => file,
        Err(e) => {
            return fail(
                common,
                &format!("cannot exclusively own this repo's integrator: {e}"),
            );
        }
    };

    // B12: one integrator per repo. This is a genuinely exclusive act — two
    // integrators would race to move the same branch — so it takes a service
    // lease rather than trusting everyone to take turns.
    let leases = LeaseStore::at(store.root());
    let service = format!("sluice:{repo_root}");
    let resource = match LeaseResource::service(&ctx.host, &service) {
        Ok(r) => r,
        Err(e) => return fail(common, &format!("invalid service name: {e}")),
    };
    let metadata = harbard_core::LeaseMetadata {
        session: ctx.session.clone(),
        harness: ctx.harness.clone(),
        pid: Some(std::process::id()),
        parent: ctx.parent.clone(),
        repository_id: Some(repo_root.clone()),
        branch: None,
        head_sha: None,
    };
    let claim = match leases.claim(
        &resource,
        &metadata,
        900,
        SystemLeaseClock,
        &SystemPidChecker,
    ) {
        Ok(harbard_core::LeaseClaim::Acquired(l) | harbard_core::LeaseClaim::Renewed(l)) => l,
        Ok(harbard_core::LeaseClaim::Conflict(holder)) => {
            return fail(
                common,
                &format!(
                    "another integrator is draining this repo (until {})",
                    holder.expires_at_ms
                ),
            );
        }
        Err(e) => return fail(common, &format!("could not take the integrator lease: {e}")),
    };

    let mut landed = 0usize;
    let mut results = Vec::new();
    let mut record_errors = Vec::new();
    for delivery in repository.pending(store.root()) {
        let receipt = integrate_one(&repository, store, &ctx, &delivery, now);
        if receipt.verdict == Verdict::Landed {
            landed += 1;
        }
        if let Err(error) = sluice::record(store.root(), &receipt) {
            record_errors.push(format!(
                "could not persist receipt {}: {error}",
                receipt.delivery_id
            ));
        }
        results.push(receipt);
    }

    let _ = leases.release_owned(
        &resource,
        &claim.claim_id,
        &metadata,
        now,
        &SystemPidChecker,
    );

    if common.json {
        println!(
            "{}",
            json!({ "status": if record_errors.is_empty() { "drained" } else { "receipt_error" }, "landed": landed, "receipts": results, "errors": record_errors })
        );
    } else if results.is_empty() {
        println!("nothing queued for {repo_root}");
    } else {
        for r in &results {
            println!(
                "{:9} {} → {}{}",
                r.verdict.as_str(),
                &r.delivered_sha[..7.min(r.delivered_sha.len())],
                r.branch,
                if r.detail.is_empty() {
                    String::new()
                } else {
                    format!("  ({})", sluice::tail(&r.detail, 120).replace('\n', " · "))
                }
            );
        }
        println!("{landed}/{} landed", results.len());
    }
    for error in &record_errors {
        eprintln!("harbard: {error}");
    }
    i32::from(!record_errors.is_empty())
}

/// Build the candidate, check it, and land exactly it — or nothing.
fn integrate_one(
    repository: &Repository,
    store: &Store,
    ctx: &Context,
    d: &Delivery,
    now: u64,
) -> Receipt {
    let repo = repository.root.as_path();
    let mut receipt = Receipt {
        delivery_id: d.id.clone(),
        repo: d.repo.clone(),
        branch: d.branch.clone(),
        delivered_sha: d.sha.clone(),
        onto: String::new(),
        candidate: None,
        verdict: Verdict::Failed,
        detail: String::new(),
        session: ctx.session.clone(),
        decided_ms: now,
    };

    // The tip as it is NOW, not the base the agent claimed. That difference is
    // the whole point: the delivery was built against a world that has moved.
    let tip = match resolve(repo, &d.branch) {
        Ok(t) => t,
        Err(e) => {
            receipt.detail = format!("cannot resolve {}: {e}", d.branch);
            return receipt;
        }
    };
    receipt.onto = tip.clone();

    // The candidate is built in a throwaway worktree so a failure — or being
    // killed — cannot leave the operator's tree half-merged. B10.
    if !harbard_core::is_event_id(&d.id) {
        receipt.detail = "delivery ID must be a full event ID".into();
        return receipt;
    }
    let work = repository.common_dir.join("harbard-sluice").join(&d.id);
    let _ = std::fs::remove_dir_all(&work);
    if let Err(e) = git(
        repo,
        &[
            "worktree",
            "add",
            "--detach",
            work.to_str().unwrap_or_default(),
            &tip,
        ],
    ) {
        receipt.detail = format!("could not create the candidate worktree: {e}");
        return receipt;
    }

    let result = build_and_check(&work, d, &mut receipt);

    // Always clean up: the worktree is scaffolding, and leaving it behind would
    // put a second writer domain inside the repo we are protecting.
    let _ = git(
        repo,
        &[
            "worktree",
            "remove",
            "--force",
            work.to_str().unwrap_or_default(),
        ],
    );
    let _ = std::fs::remove_dir_all(&work);

    if let Some(candidate) = result {
        // B7: fast-forward to the exact commit that was checked, and only if
        // the tip is still where it was when we built it. If someone moved the
        // branch while the checks ran, this attempt fails with a receipt
        // rather than landing something nobody verified. The owner can deliver again.
        match resolve(repo, &d.branch) {
            Ok(still) if still == tip => {
                match land_candidate(repository, store, ctx, &d.branch, &tip, &candidate) {
                    Ok(_) => {
                        receipt.verdict = Verdict::Landed;
                        receipt.candidate = Some(candidate);
                    }
                    Err(e) => {
                        receipt.verdict = Verdict::Failed;
                        receipt.detail = format!("checks passed but landing failed: {e}");
                    }
                }
            }
            Ok(moved) => {
                receipt.verdict = Verdict::Failed;
                receipt.detail = format!(
                    "{} moved from {} to {} while the checks ran; not landing an unverified merge",
                    d.branch,
                    &tip[..7.min(tip.len())],
                    &moved[..7.min(moved.len())]
                );
            }
            Err(e) => {
                receipt.verdict = Verdict::Failed;
                receipt.detail = format!("cannot re-read {}: {e}", d.branch);
            }
        }
    }
    receipt
}

/// Merge into the candidate worktree and run the checks there.
/// Returns the candidate SHA when it is safe to land.
fn build_and_check(work: &Path, d: &Delivery, receipt: &mut Receipt) -> Option<String> {
    // Repository hooks must see its configured identity, never an invented
    // integration user or an identity inferred from the host.
    let merged = git(
        work,
        &[
            "-c",
            "user.useConfigOnly=true",
            "merge",
            "--no-ff",
            "-m",
            &format!("sluice: {} onto {}", &d.sha[..7.min(d.sha.len())], d.branch),
            &d.sha,
        ],
    );
    if let Err(e) = merged {
        // B9: a conflict is an answer, not a crash. The queue moves on.
        receipt.verdict = Verdict::Conflict;
        receipt.detail = sluice::tail(&e, 600);
        return None;
    }

    let candidate = match resolve(work, "HEAD") {
        Ok(c) => c,
        Err(e) => {
            receipt.detail = format!("candidate has no HEAD: {e}");
            return None;
        }
    };
    receipt.candidate = Some(candidate.clone());

    // A worktree is not an environment. Whatever the delivery says it needs to
    // become runnable happens here, and if THAT fails the verdict is `failed`,
    // never `rejected`: the commit was never given a chance to be wrong.
    if !d.prepare.trim().is_empty() {
        match Command::new("sh")
            .arg("-lc")
            .arg(&d.prepare)
            .current_dir(work)
            .output()
        {
            Ok(o) if o.status.success() => {}
            Ok(o) => {
                receipt.verdict = Verdict::Failed;
                let mut text = String::from("preparation failed, so the checks never ran: ");
                text.push_str(&String::from_utf8_lossy(&o.stdout));
                text.push_str(&String::from_utf8_lossy(&o.stderr));
                receipt.detail = sluice::tail(&text, 900);
                return None;
            }
            Err(e) => {
                receipt.verdict = Verdict::Failed;
                receipt.detail = format!("preparation could not run: {e}");
                return None;
            }
        }
    }

    if let Err(error) = unchanged_candidate(work, &candidate, "preparation") {
        receipt.detail = error;
        return None;
    }

    if d.checks.trim().is_empty() {
        receipt.detail = "no checks requested".into();
        return Some(candidate);
    }

    // B6: the checks run in the candidate worktree. Running them anywhere else
    // would verify something other than what lands.
    let out = Command::new("sh")
        .arg("-lc")
        .arg(&d.checks)
        .current_dir(work)
        .output();
    match out {
        Ok(o) if o.status.success() => {
            if let Err(error) = unchanged_candidate(work, &candidate, "checks") {
                receipt.detail = error;
                return None;
            }
            receipt.detail = sluice::tail(&String::from_utf8_lossy(&o.stdout), 400);
            Some(candidate)
        }
        Ok(o) => {
            receipt.verdict = Verdict::Rejected;
            let mut text = String::from_utf8_lossy(&o.stdout).to_string();
            text.push_str(&String::from_utf8_lossy(&o.stderr));
            receipt.detail = sluice::tail(&text, 900);
            None
        }
        Err(e) => {
            receipt.detail = format!("checks could not run: {e}");
            None
        }
    }
    // Deliberately no rebase, no retry, no second build after this point:
    // whatever the checks ran on is what lands.
}

fn unchanged_candidate(work: &Path, candidate: &str, stage: &str) -> Result<(), String> {
    let head = resolve(work, "HEAD")?;
    let status = git(work, &["status", "--porcelain", "--untracked-files=no"])?;
    if head != candidate || !status.is_empty() {
        return Err(format!(
            "{stage} modified the candidate; refusing to land content different from what was checked"
        ));
    }
    Ok(())
}

/// `harbard deliveries` — the queue and its receipts.
pub fn cmd_deliveries(store: &Store, common: &Common, repo: &str) -> i32 {
    let repository = match Repository::resolve(repo) {
        Ok(r) => r,
        Err(e) => return fail(common, &format!("not a Git repository: {e}")),
    };
    let repo_root = &repository.identity;
    let pending = repository.pending(store.root());
    let receipts = repository.receipts(store.root());
    if common.json {
        println!(
            "{}",
            json!({ "repo": repo_root, "pending": pending, "receipts": receipts })
        );
        return 0;
    }
    if pending.is_empty() && receipts.is_empty() {
        println!("no deliveries for {repo_root}");
        return 0;
    }
    for d in &pending {
        println!(
            "pending  {} · {} onto {} · by {}",
            &d.id[..8.min(d.id.len())],
            &d.sha[..7.min(d.sha.len())],
            d.branch,
            d.harness
        );
    }
    for r in receipts.iter().rev().take(10) {
        println!(
            "{:9} {} onto {}{}",
            r.verdict.as_str(),
            &r.delivered_sha[..7.min(r.delivered_sha.len())],
            r.branch,
            r.candidate
                .as_ref()
                .map(|c| format!(" → {}", &c[..7.min(c.len())]))
                .unwrap_or_default()
        );
    }
    0
}

fn fail(common: &Common, message: &str) -> i32 {
    if common.json {
        println!("{}", json!({ "status": "error", "error": message }));
    } else {
        eprintln!("harbard: {message}");
    }
    1
}
