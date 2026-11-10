//! The shell's profile and update stores, copied unchanged from the shell
//! so Preferences and the dialogs can be built and tried on their own.
//! Once the shell is in, this module becomes
//! `pub use crate::shell::{profile, update};` and the two files go.
#![allow(dead_code, clippy::all)]

pub mod profile;
pub mod update;

pub fn init(cx: &mut gpui::App) {
    profile::init(cx);
    update::init(cx);
}
