//! A fingerprint of the layered icon's sources, so a stale `Assets.car`
//! cannot ship. `scripts/render-dock.py` writes it to
//! `icons/Assets.car.sources` each time actool compiles the car; `build.rs`
//! refuses to build when the sources no longer match it.
//!
//! FNV-1a 64 over each source's name and bytes, in a fixed order. The same
//! few lines live in render-dock.py (`layered_stamp`).

use std::path::Path;

/// The sources, relative to `assets/brand/harbard.icon`.
pub const SOURCES: [&str; 3] = ["icon.json", "Assets/bubble.svg", "Assets/dot.svg"];

pub fn stamp(icon_dir: &Path) -> std::io::Result<String> {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    let mut eat = |bytes: &[u8]| {
        for b in bytes {
            hash ^= u64::from(*b);
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
    };
    for name in SOURCES {
        eat(name.as_bytes());
        eat(&[0]);
        eat(&std::fs::read(icon_dir.join(name))?);
        eat(&[0]);
    }
    Ok(format!("fnv1a64:{hash:016x}"))
}
