//! The commit this binary is built from, as `HARBARD_COMMIT` (empty when
//! git cannot say): `/api/v1/meta` reports it, and "Deliveries" compares a
//! delivery against it to say "Installed". `HARBARD_COMMIT` in the build's
//! environment overrides it (a build from a tarball, a reproducible build).
//!
//! Also the checkout it is built from, as `HARBARD_SOURCE`: updates
//! (docs/SPEC-UPDATES.md) fetch and fast-forward that repository's primary
//! checkout. Empty when the source is not a git checkout.

use std::path::PathBuf;
use std::process::Command;

fn git(args: &[&str]) -> Option<String> {
    let out = Command::new("git")
        .args(args)
        .env("GIT_OPTIONAL_LOCKS", "0")
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
        .filter(|s| !s.is_empty())
}

fn main() {
    println!("cargo:rerun-if-env-changed=HARBARD_COMMIT");
    let commit = std::env::var("HARBARD_COMMIT")
        .ok()
        .filter(|c| !c.is_empty())
        .or_else(|| git(&["rev-parse", "HEAD"]))
        .unwrap_or_default();
    println!("cargo:rustc-env=HARBARD_COMMIT={commit}");
    println!("cargo:rerun-if-env-changed=HARBARD_SOURCE");
    let source = std::env::var("HARBARD_SOURCE")
        .ok()
        .filter(|s| !s.is_empty())
        .or_else(|| git(&["rev-parse", "--show-toplevel"]))
        .unwrap_or_default();
    println!("cargo:rustc-env=HARBARD_SOURCE={source}");
    // Rebuilt when HEAD moves: the worktree's HEAD, the branch it points
    // to, and packed refs. Only files that exist, or cargo reruns always.
    let mut watch: Vec<PathBuf> = Vec::new();
    if let Some(dir) = git(&["rev-parse", "--absolute-git-dir"]) {
        let dir = PathBuf::from(dir);
        watch.push(dir.join("HEAD"));
        if let Some(common) = git(&["rev-parse", "--path-format=absolute", "--git-common-dir"]) {
            let common = PathBuf::from(common);
            watch.push(common.join("packed-refs"));
            if let Some(branch) = git(&["symbolic-ref", "-q", "HEAD"]) {
                watch.push(common.join(branch));
            }
        }
    }
    for path in watch.into_iter().filter(|p| p.exists()) {
        println!("cargo:rerun-if-changed={}", path.display());
    }
}
