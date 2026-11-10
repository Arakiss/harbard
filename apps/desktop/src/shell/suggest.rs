//! The desktop's away suggestion (`suggest-card.tsx`, `lib/suggest.ts`): a
//! corner card; only the operator's click changes the state.

use gpui::{App, Context, IntoElement, Render, Window, div};

/// What the desktop's away watch saw.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Suggestion {
    /// Ten minutes away from the Mac, or a locked screen.
    Away,
    /// Input again after being away.
    Available,
}

pub fn show(s: Suggestion, cx: &mut App) {
    let _ = (s, cx);
}

pub struct SuggestCard;

impl SuggestCard {
    pub fn new(_window: &mut Window, _cx: &mut Context<Self>) -> Self {
        SuggestCard
    }
}

impl Render for SuggestCard {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
    }
}
