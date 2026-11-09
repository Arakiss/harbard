//! `harbard hub rotate`: a new shared hub token, 0600, and what to restart.
//! Everything runs under a throwaway home; the real `~/.harbard` is never read.
//!
//! A token that reached the wrong place must be replaceable without touching
//! per-device credentials.

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

static SEQ: AtomicU64 = AtomicU64::new(0);
const OLD: &str = "old-token-fixture-not-a-secret";

struct Home(PathBuf);

impl Drop for Home {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

impl Home {
    fn new() -> Self {
        let n = SEQ.fetch_add(1, Ordering::Relaxed);
        let root =
            std::env::temp_dir().join(format!("harbard-hub-rotate-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("store")).unwrap();
        Home(root)
    }

    fn store(&self) -> PathBuf {
        self.0.join("store")
    }

    fn token(&self) -> PathBuf {
        self.store().join("hub.token")
    }

    fn rotate(&self, env: &[(&str, &str)], json: bool) -> Output {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_harbard"));
        cmd.env_clear()
            .env("HOME", &self.0)
            .env("PATH", "/usr/bin:/bin")
            .env("HARBARD_API", "http://127.0.0.1:9")
            .envs(env.iter().copied())
            .args(["hub", "rotate", "--home"])
            .arg(self.store());
        if json {
            cmd.arg("--json");
        }
        cmd.output().unwrap()
    }
}

fn mode(path: &Path) -> u32 {
    std::fs::metadata(path).unwrap().permissions().mode() & 0o777
}

#[test]
fn rotate_replaces_the_token_0600_and_leaves_credentials_untouched() {
    let home = Home::new();
    std::fs::write(home.token(), format!("{OLD}\n")).unwrap();
    std::fs::set_permissions(home.token(), std::fs::Permissions::from_mode(0o644)).unwrap();
    let credentials = home.store().join("credentials.json");
    let fake = r#"{"devices":{"laptop":{"sha256":"0000000000000000000000000000000000000000000000000000000000000000"}}}"#;
    std::fs::write(&credentials, fake).unwrap();
    std::fs::set_permissions(&credentials, std::fs::Permissions::from_mode(0o600)).unwrap();
    let before = std::fs::metadata(&credentials).unwrap().modified().unwrap();

    let out = home.rotate(&[], false);
    assert!(out.status.success(), "{out:?}");
    let new = std::fs::read_to_string(home.token()).unwrap();
    let new = new.trim();
    assert_ne!(new, OLD);
    assert!(new.len() >= 52, "two ULIDs: {}", new.len());
    assert_eq!(mode(&home.token()), 0o600);
    assert_eq!(std::fs::read_to_string(&credentials).unwrap(), fake);
    assert_eq!(
        std::fs::metadata(&credentials).unwrap().modified().unwrap(),
        before
    );
    assert_eq!(mode(&credentials), 0o600);

    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        !stdout.contains(new),
        "the token is never printed: {stdout}"
    );
    assert!(
        stdout.contains("credentials.json left untouched"),
        "{stdout}"
    );
    assert!(stdout.contains("com.harbard.serve"), "{stdout}");
    assert!(stdout.contains("old one stops working"), "{stdout}");
    // No leftover temp file beside it.
    let names: Vec<String> = std::fs::read_dir(home.store())
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert!(names.iter().all(|n| !n.contains("tmp")), "{names:?}");
}

#[test]
fn without_credentials_every_paired_laptop_is_told_to_pair_again() {
    let home = Home::new();
    std::fs::write(home.token(), format!("{OLD}\n")).unwrap();
    let out = home.rotate(&[], false);
    assert!(out.status.success(), "{out:?}");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("must pair again"), "{stdout}");
    assert!(stdout.contains("harbard hub invite"), "{stdout}");
    assert!(!home.store().join("credentials.json").exists());
}

#[test]
fn rotate_says_what_it_did_as_json_and_warns_about_an_env_override() {
    let home = Home::new();
    let out = home.rotate(&[("HARBARD_HUB_TOKEN", "env-fixture")], true);
    assert!(out.status.success(), "{out:?}");
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["replaced"], false, "{v}");
    assert_eq!(v["credentials_kept"], false, "{v}");
    assert_eq!(v["env_override"], true, "{v}");
    assert_eq!(mode(&home.token()), 0o600);
    let token = std::fs::read_to_string(home.token()).unwrap();
    assert!(!String::from_utf8_lossy(&out.stdout).contains(token.trim()));
    // Twice: a different token each time.
    let again = home.rotate(&[], true);
    assert!(again.status.success(), "{again:?}");
    assert_ne!(std::fs::read_to_string(home.token()).unwrap(), token);
}
