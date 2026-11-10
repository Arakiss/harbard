//! Markdown inside a view. The console used its shared `MarkdownView`;
//! until the app has one, GPUI Kit's Markdown view draws it, with remote
//! images turned off so a view never makes the app fetch anything.

use super::build::{B, eid};
use gpui::{
    AnyElement, ImageSource, IntoElement, ParentElement, Resource, SharedString, Styled, div,
};
use gpui_kit::component::text::TextView;
use std::path::PathBuf;
use std::sync::Arc;

/// Above this the console showed the text plain.
const MARKDOWN_MAX: usize = 1_000_000;

pub(super) fn view(b: &mut B<'_, '_>, source: &str, id: &str) -> AnyElement {
    if source.chars().count() > MARKDOWN_MAX {
        return div()
            .whitespace_normal()
            .child(source.to_string())
            .into_any_element();
    }
    let inert =
        |_: &gpui::SharedUri| ImageSource::Resource(Resource::Path(Arc::from(PathBuf::new())));
    let _ = b;
    div()
        .min_w_0()
        .child(
            TextView::markdown(
                eid(format!("{id}-md")),
                SharedString::from(source.to_string()),
            )
            .image_source(inert),
        )
        .into_any_element()
}
