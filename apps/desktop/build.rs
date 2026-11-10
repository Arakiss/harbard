//! Two things the app's build owns:
//!
//! - `HARBARD_APP_COMMIT`: the short commit the app is built from. The hub
//!   passes it to serve's workspace read, so "Deliveries" can say a delivery
//!   is installed in the app too. `HARBARD_APP_COMMIT` in the build's
//!   environment overrides git (a build from a tarball, a reproducible build).
//! - The layered icon check: `bundle/Assets.car` is compiled by actool from
//!   `assets/brand/harbard.icon` and committed. With `CFBundleIconName` set,
//!   macOS draws the car and never the .icns, so a car older than its sources
//!   must not reach a bundle.

use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    commit();
    check_layered_icon();
}

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

fn commit() {
    println!("cargo:rerun-if-env-changed=HARBARD_APP_COMMIT");
    let commit = std::env::var("HARBARD_APP_COMMIT")
        .ok()
        .filter(|c| !c.is_empty())
        .or_else(|| git(&["rev-parse", "--short", "HEAD"]));
    if let Some(commit) = commit {
        println!("cargo:rustc-env=HARBARD_APP_COMMIT={commit}");
    }
    // Rebuilt when HEAD moves: the worktree's HEAD, the branch it points to,
    // and packed refs. Only files that exist, or cargo reruns every build.
    let mut watch: Vec<PathBuf> = Vec::new();
    if let Some(dir) = git(&["rev-parse", "--absolute-git-dir"]) {
        watch.push(PathBuf::from(dir).join("HEAD"));
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

/// The sources, relative to `assets/brand/harbard.icon`. The same list and
/// fingerprint live in `scripts/render-dock.py` (`layered_stamp`), which
/// writes `bundle/Assets.car.sources` each time actool compiles the car.
const LAYERED_SOURCES: [&str; 3] = ["icon.json", "Assets/bubble.svg", "Assets/dot.svg"];

/// FNV-1a 64 over each source's name and bytes, in a fixed order.
fn layered_stamp(icon_dir: &Path) -> std::io::Result<String> {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    let mut eat = |bytes: &[u8]| {
        for b in bytes {
            hash ^= u64::from(*b);
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
    };
    for name in LAYERED_SOURCES {
        eat(name.as_bytes());
        eat(&[0]);
        eat(&std::fs::read(icon_dir.join(name))?);
        eat(&[0]);
    }
    Ok(format!("fnv1a64:{hash:016x}"))
}

fn check_layered_icon() {
    let icon = Path::new("../../assets/brand/harbard.icon");
    let recorded = Path::new("bundle/Assets.car.sources");
    println!("cargo:rerun-if-changed={}", recorded.display());
    for name in LAYERED_SOURCES {
        println!("cargo:rerun-if-changed={}", icon.join(name).display());
    }
    let current = layered_stamp(icon).expect("read assets/brand/harbard.icon");
    let recorded = std::fs::read_to_string(recorded).unwrap_or_default();
    assert_eq!(
        recorded.trim(),
        current,
        "bundle/Assets.car is stale: its sources changed since it was compiled. \
         Run scripts/render-brand.sh on a Mac with Xcode, and commit the car."
    );
}
