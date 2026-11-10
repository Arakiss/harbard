//! Calm mode (`calm-screen.tsx`, `lib/calm.ts`): while Swamped the panel
//! shows only the one thing that needs the operator now.

use gpui::{Context, IntoElement, Render, Window, div};

/// On in Swamped, unless they left it for this Swamped (`exited` is that Swamped's start).
pub fn calm_on(state: &str, since: u64, exited: Option<u64>) -> bool {
    state == "swamped" && exited != Some(since)
}

pub struct CalmScreen;

impl CalmScreen {
    pub fn new(_window: &mut Window, _cx: &mut Context<Self>) -> Self {
        CalmScreen
    }
}

impl Render for CalmScreen {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
    }
}
