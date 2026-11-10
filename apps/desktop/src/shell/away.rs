//! "While you were away" (`away-card.tsx`, `lib/away.ts`): a modal over the
//! shell when the profile store holds an [`super::profile::Away`].

use gpui::{Context, IntoElement, Render, Window, div};

pub struct AwayCard;

impl AwayCard {
    pub fn new(_window: &mut Window, _cx: &mut Context<Self>) -> Self {
        AwayCard
    }
}

impl Render for AwayCard {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
    }
}
