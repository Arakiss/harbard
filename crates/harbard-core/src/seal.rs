//! What serve minted: an HMAC-SHA256 over an event, with a key only serve
//! creates.
//!
//! A rule reaches every session's protocol card and SessionStart brief.
//! Before this, any line on the bus that said `"harness":"human",
//! "role":"operator","event":"rule"` was a rule, and so was any row put into
//! the feedback index. Now a rule counts only when its payload carries a
//! MAC made with `{harbard home}/serve.key` over the event id and the
//! payload. A line or a row without a valid MAC is ignored.
//!
//! # What this does not do
//!
//! The key is a 0600 file of the operator's own user, and the hooks that
//! build the brief run as that user and must read it to verify. A process of
//! that same user, an agent among them, can read the key, or edit
//! `workspace.json` and `profile.json` directly. Nothing inside one macOS
//! user separates the operator from their own processes.
//!
//! What it does buy:
//! - a bus line or an index row that someone appended is no longer a rule;
//! - a satellite never holds the key, so a satellite cannot make one;
//! - forging one now means reading a secret on purpose, which the harness
//!   guards can refuse, rather than writing one JSON line.
//!
//! The "Was it you?" cards ([`crate::workspace::DeskCheck`]) are the
//! backstop: every approval, every change of "Context" and every switch-on
//! of the memory sink asks the operator, with Revoke at hand.

use sha2::{Digest, Sha256};
use std::fs;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

/// The key's file in the harbard home.
pub const KEY_FILE: &str = "serve.key";
/// Bytes of key.
const KEY_BYTES: usize = 32;

/// The serve key.
#[derive(Clone, PartialEq, Eq)]
pub struct Key([u8; KEY_BYTES]);

impl std::fmt::Debug for Key {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Key(…)")
    }
}

pub fn path(root: &Path) -> PathBuf {
    root.join(KEY_FILE)
}

/// `n` random bytes from the OS.
pub fn random_bytes(n: usize) -> io::Result<Vec<u8>> {
    let mut out = vec![0u8; n];
    fs::File::open("/dev/urandom")?.read_exact(&mut out)?;
    Ok(out)
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn unhex(s: &str) -> Option<Vec<u8>> {
    let s = s.trim();
    if !s.len().is_multiple_of(2) {
        return None;
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(s.get(i..i + 2)?, 16).ok())
        .collect()
}

/// SHA-256 of `parts`, each closed by a NUL, as hex: what a one-time code
/// is bound to.
pub fn digest_hex(parts: &[&str]) -> String {
    let mut h = Sha256::new();
    for p in parts {
        h.update(p.as_bytes());
        h.update([0u8]);
    }
    hex(&h.finalize())
}

/// `n` random bytes as hex: a one-time code.
pub fn random_hex(n: usize) -> io::Result<String> {
    random_bytes(n).map(|b| hex(&b))
}

/// The key, when serve has made one. Reading never creates it: a missing or
/// malformed key verifies nothing, so no rule counts.
pub fn load(root: &Path) -> Option<Key> {
    let raw = fs::read_to_string(path(root)).ok()?;
    let bytes = unhex(&raw)?;
    let key: [u8; KEY_BYTES] = bytes.try_into().ok()?;
    Some(Key(key))
}

/// The key, made on first use: 0600 from its first byte, never replaced.
/// Returns whether it was made now.
pub fn ensure(root: &Path) -> io::Result<(Key, bool)> {
    if let Some(k) = load(root) {
        return Ok((k, false));
    }
    fs::create_dir_all(root)?;
    let bytes = random_bytes(KEY_BYTES)?;
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    match options.open(path(root)) {
        Ok(mut f) => {
            f.write_all(format!("{}\n", hex(&bytes)).as_bytes())?;
            f.sync_all()?;
        }
        // Another writer made it first: use theirs.
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
        Err(e) => return Err(e),
    }
    let key = load(root).ok_or_else(|| io::Error::other("serve.key is unreadable"))?;
    let made = key.0[..] == bytes[..];
    Ok((key, made))
}

fn hmac_sha256(key: &[u8], msg: &[u8]) -> [u8; 32] {
    const BLOCK: usize = 64;
    let mut k = [0u8; BLOCK];
    if key.len() > BLOCK {
        k[..32].copy_from_slice(&Sha256::digest(key));
    } else {
        k[..key.len()].copy_from_slice(key);
    }
    let mut inner = Sha256::new();
    inner.update(k.map(|b| b ^ 0x36));
    inner.update(msg);
    let inner = inner.finalize();
    let mut outer = Sha256::new();
    outer.update(k.map(|b| b ^ 0x5c));
    outer.update(inner);
    outer.finalize().into()
}

fn message(domain: &str, id: &str, body: &str) -> Vec<u8> {
    format!("{domain}\n{id}\n{body}").into_bytes()
}

/// The MAC of `body` for the event `id`, under `domain` (`harbard-rule-v1`).
pub fn mac(key: &Key, domain: &str, id: &str, body: &str) -> String {
    hex(&hmac_sha256(&key.0, &message(domain, id, body)))
}

/// Whether `tag` is the MAC of `body` for `id`, compared in constant time.
pub fn verify(key: &Key, domain: &str, id: &str, body: &str, tag: &str) -> bool {
    let Some(given) = unhex(tag) else {
        return false;
    };
    let want = hmac_sha256(&key.0, &message(domain, id, body));
    given.len() == want.len()
        && given
            .iter()
            .zip(want)
            .fold(0u8, |acc, (a, b)| acc | (a ^ b))
            == 0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::TempHome;

    /// RFC 4231, test case 2.
    #[test]
    fn hmac_matches_the_rfc_vector() {
        let out = hmac_sha256(b"Jefe", b"what do ya want for nothing?");
        assert_eq!(
            hex(&out),
            "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843"
        );
    }

    #[test]
    fn the_key_is_made_once_0600_and_verifies_only_its_own_macs() {
        let home = TempHome::new();
        assert!(load(home.path()).is_none(), "reading never creates it");
        let (key, made) = ensure(home.path()).unwrap();
        assert!(made);
        let (again, made) = ensure(home.path()).unwrap();
        assert!(!made);
        assert_eq!(key, again);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(path(home.path()))
                .unwrap()
                .permissions()
                .mode()
                & 0o777;
            assert_eq!(mode, 0o600);
        }
        let tag = mac(&key, "d", "id1", "body");
        assert!(verify(&key, "d", "id1", "body", &tag));
        assert!(!verify(&key, "d", "id2", "body", &tag), "bound to the id");
        assert!(
            !verify(&key, "d", "id1", "body!", &tag),
            "bound to the body"
        );
        assert!(
            !verify(&key, "e", "id1", "body", &tag),
            "bound to the domain"
        );
        assert!(!verify(&key, "d", "id1", "body", "zz"), "malformed");
        let other = TempHome::new();
        let (other_key, _) = ensure(other.path()).unwrap();
        assert!(!verify(&other_key, "d", "id1", "body", &tag), "another key");
    }
}
