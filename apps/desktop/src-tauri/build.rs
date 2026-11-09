#[path = "src/layered_stamp.rs"]
mod layered_stamp;

use std::path::Path;

fn main() {
    check_layered_icon();
    tauri_build::build()
}

/// `icons/Assets.car` is compiled by actool from `assets/brand/harbard.icon`
/// and committed. With `CFBundleIconName` set, macOS draws the car and never
/// the .icns, so a car older than its sources must not reach a bundle.
fn check_layered_icon() {
    let icon = Path::new("../../../assets/brand/harbard.icon");
    let recorded = Path::new("icons/Assets.car.sources");
    println!("cargo:rerun-if-changed={}", recorded.display());
    for name in layered_stamp::SOURCES {
        println!("cargo:rerun-if-changed={}", icon.join(name).display());
    }
    let current = layered_stamp::stamp(icon).expect("read assets/brand/harbard.icon");
    let recorded = std::fs::read_to_string(recorded).unwrap_or_default();
    assert_eq!(
        recorded.trim(),
        current,
        "icons/Assets.car is stale: its sources changed since it was compiled. \
         Run scripts/render-brand.sh on a Mac with Xcode, and commit the car."
    );
}
