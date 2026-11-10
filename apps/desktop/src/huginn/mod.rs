//! Huginn views: `.huginn` JSON attachments rendered natively. Owned by the
//! huginn lane; the chat lane embeds [`HuginnView`] for each such attachment.
//! The constructor's signature is the contract.
//!
//! A view is fetched once from serve, checked by the same validator the CLI
//! runs (`harbard_cli::huginn`), and drawn with its own state store. Its
//! answers are ordinary operator replies (`POST /workspace/reply`) in the
//! text format agents parse (`answer.rs`).

pub mod answer;
mod ask;
mod build;
mod charts;
mod debug;
mod echo;
pub mod expr;
mod md;
pub mod scale;
pub mod spec;

pub use debug::open_debug_window;
#[allow(unused_imports)] // the chat lane's entry points for answer notes
pub use echo::{AnswerEcho, operator_answer};

use crate::api;
use crate::desk::Desk;
use crate::i18n::{I18n, t};
use crate::model::{Attachment, Message};
use crate::theme::Theme;
use crate::ui;
use answer::DescribedLine;
use build::{B, Pal, Scope, Target, Ui};
use gpui::prelude::FluentBuilder as _;
use gpui::{
    AppContext, Context, FontWeight, InteractiveElement as _, IntoElement, ParentElement, Render, StatefulInteractiveElement as _, Styled,
    Subscription, Window, div, px, relative,
};
use harbard_cli::huginn::Issue;
use serde_json::Value;
use spec::Spec;
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, OnceLock};

/// An answer already given to one Choice or Form.
#[derive(Debug, Clone, PartialEq)]
pub struct Given {
    pub lines: Vec<DescribedLine>,
    pub ts_ms: Option<u64>,
}

/// An image a view shows, by attachment id.
#[derive(Clone)]
pub(crate) enum ImageLoad {
    Loading,
    Failed,
    Ready(Arc<gpui::Image>, u32, u32),
}

enum Load {
    Loading,
    /// The file could not be fetched.
    Unreadable,
    Refused(Vec<Issue>),
    Ready(Arc<Spec>),
}

type Parsed = Result<Arc<Spec>, Vec<Issue>>;

/// Views never change once attached, so each is fetched and checked once.
fn views() -> &'static Mutex<HashMap<String, Parsed>> {
    static CACHE: OnceLock<Mutex<HashMap<String, Parsed>>> = OnceLock::new();
    CACHE.get_or_init(Default::default)
}

fn images() -> &'static Mutex<HashMap<String, (Arc<gpui::Image>, u32, u32)>> {
    static CACHE: OnceLock<Mutex<HashMap<String, (Arc<gpui::Image>, u32, u32)>>> = OnceLock::new();
    CACHE.get_or_init(Default::default)
}

/// `GET /attachments/<ulid>/<name>`, each segment encoded.
pub(crate) fn attachment_path(id: &str) -> String {
    let segments: Vec<String> = id.split('/').map(api::enc).collect();
    format!("/attachments/{}", segments.join("/"))
}

/// Fetch and check a view (blocking; run it off the UI thread).
pub(crate) fn fetch_view(id: &str) -> Option<Parsed> {
    if let Some(hit) = views().lock().ok().and_then(|c| c.get(id).cloned()) {
        return Some(hit);
    }
    let bytes = api::get_bytes(&attachment_path(id)).ok()?;
    let parsed = Spec::parse(&bytes).map(Arc::new);
    if let Ok(mut cache) = views().lock() {
        cache.insert(id.to_string(), parsed.clone());
    }
    Some(parsed)
}

fn image_format(bytes: &[u8]) -> Option<gpui::ImageFormat> {
    use gpui::ImageFormat::*;
    if bytes.starts_with(b"\x89PNG") {
        Some(Png)
    } else if bytes.starts_with(&[0xFF, 0xD8]) {
        Some(Jpeg)
    } else if bytes.starts_with(b"GIF8") {
        Some(Gif)
    } else if bytes.len() > 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        Some(Webp)
    } else if bytes.starts_with(b"BM") {
        Some(Bmp)
    } else {
        None
    }
}

/// Fetch an image attachment and read its size (blocking).
fn fetch_image(id: &str) -> Option<(Arc<gpui::Image>, u32, u32)> {
    if let Some(hit) = images().lock().ok().and_then(|c| c.get(id).cloned()) {
        return Some(hit);
    }
    let bytes = api::get_bytes(&attachment_path(id)).ok()?;
    let format = image_format(&bytes)?;
    let dims = imagesize::blob_size(&bytes).ok()?;
    let (w, h) = (dims.width as u32, dims.height as u32);
    let out = (Arc::new(gpui::Image::from_bytes(format, bytes)), w.max(1), h.max(1));
    if let Ok(mut cache) = images().lock() {
        cache.insert(id.to_string(), out.clone());
    }
    Some(out)
}

/// One rendered view, with its own form state. Create once per attachment:
/// `cx.new(|cx| HuginnView::new(attachment.clone(), message.clone(), window, cx))`.
pub struct HuginnView {
    pub attachment: Attachment,
    pub message: Message,
    load: Load,
    /// The view's state store: a copy of `spec.state`, never persisted.
    state: Value,
    ui: Ui,
    /// Answers on the bus, by Choice or Form id.
    given: HashMap<String, Given>,
    subs: Vec<Subscription>,
}

impl HuginnView {
    pub fn new(attachment: Attachment, message: Message, _window: &mut Window, cx: &mut Context<Self>) -> Self {
        let desk = Desk::get(cx);
        let subs = vec![
            cx.observe(&desk, |this, _, cx| {
                if this.refresh_given(cx) {
                    cx.notify();
                }
            }),
            cx.observe_global::<Theme>(|_, cx| cx.notify()),
            cx.observe_global::<I18n>(|_, cx| cx.notify()),
        ];
        let mut view = HuginnView {
            attachment,
            message,
            load: Load::Loading,
            state: Value::Object(Default::default()),
            ui: Ui::default(),
            given: HashMap::new(),
            subs,
        };
        let cached = views().lock().ok().and_then(|c| c.get(&view.attachment.id).cloned());
        match cached {
            Some(parsed) => view.loaded(Some(parsed), cx),
            None => {
                let id = view.attachment.id.clone();
                cx.spawn(async move |this, cx| {
                    let parsed = cx.background_spawn(async move { fetch_view(&id) }).await;
                    this.update(cx, |view, cx| {
                        view.loaded(parsed, cx);
                        cx.notify();
                    })
                    .ok();
                })
                .detach();
            }
        }
        view
    }

    fn loaded(&mut self, parsed: Option<Parsed>, cx: &mut Context<Self>) {
        self.load = match parsed {
            None => Load::Unreadable,
            Some(Err(issues)) => Load::Refused(issues),
            Some(Ok(spec)) => {
                self.state = spec.state.clone();
                Load::Ready(spec)
            }
        };
        self.refresh_given(cx);
        self.load_images(cx);
    }

    /// The images this view names, from the message's own attachments.
    fn load_images(&mut self, cx: &mut Context<Self>) {
        let Load::Ready(spec) = &self.load else { return };
        let named = spec.files_named();
        for file in self.message.attachments.iter().filter(|a| a.kind == "image" && named.contains(&a.name)) {
            if self.ui.images.contains_key(&file.id) {
                continue;
            }
            if let Some((img, w, h)) = images().lock().ok().and_then(|c| c.get(&file.id).cloned()) {
                self.ui.images.insert(file.id.clone(), ImageLoad::Ready(img, w, h));
                continue;
            }
            self.ui.images.insert(file.id.clone(), ImageLoad::Loading);
            let id = file.id.clone();
            cx.spawn(async move |this, cx| {
                let fetch_id = id.clone();
                let got = cx.background_spawn(async move { fetch_image(&fetch_id) }).await;
                this.update(cx, |view, cx| {
                    let load = match got {
                        Some((img, w, h)) => ImageLoad::Ready(img, w, h),
                        None => ImageLoad::Failed,
                    };
                    view.ui.images.insert(id, load);
                    cx.notify();
                })
                .ok();
            })
            .detach();
        }
    }

    pub(crate) fn spec(&self) -> Option<Arc<Spec>> {
        match &self.load {
            Load::Ready(spec) => Some(spec.clone()),
            _ => None,
        }
    }

    /// Who an answer reaches; an operator's own message cannot be answered.
    fn target(&self) -> Option<Target> {
        (self.message.from != "operator").then(|| Target {
            agent: self.message.agent.clone(),
            reply_to: self.message.id.clone(),
            author: self.message.author.clone(),
        })
    }

    /// Read the answers already on the bus (`answersOf`). True when they changed.
    fn refresh_given(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(spec) = self.spec() else { return false };
        let desk = Desk::get(cx).read(cx);
        let Some(view) = desk.view.as_ref() else { return false };
        let replies: Vec<String> =
            view.message(&self.message.id).map(|m| m.replies.clone()).unwrap_or_else(|| self.message.replies.clone());
        let mut given = HashMap::new();
        if !replies.is_empty() {
            let wanted: HashSet<&str> = replies.iter().map(String::as_str).collect();
            let found: HashMap<&str, &Message> =
                view.messages.iter().filter(|m| wanted.contains(m.id.as_str())).map(|m| (m.id.as_str(), m)).collect();
            for id in &replies {
                let Some(reply) = found.get(id.as_str()) else { continue };
                if reply.from != "operator" {
                    continue;
                }
                if let Some(a) = answer::read_answer(&reply.text)
                    && a.file == self.attachment.name
                {
                    given.insert(a.id.clone(), Given { lines: answer::describe(&spec, &a), ts_ms: Some(reply.ts_ms) });
                }
            }
        }
        if given != self.given {
            self.given = given;
            return true;
        }
        false
    }

    fn refused(&mut self, issues: &[Issue], pal: &Pal, cx: &mut Context<Self>) -> gpui::AnyElement {
        let key = "#refused".to_string();
        let open = self.ui.open.get(&key).map(|e| e.1).unwrap_or(false);
        let file = self.attachment.name.clone();
        let ink3 = pal.c("ink-3");
        div()
            .mt(px(8.))
            .px(px(12.))
            .py(px(10.))
            .border_1()
            .border_dashed()
            .border_color(pal.c("line-strong"))
            .rounded(px(8.))
            .text_color(pal.c("ink-2"))
            .text_size(px(14.))
            .flex()
            .flex_col()
            .child(t(cx, "huginn.viewRefused"))
            .child(
                div()
                    .id("refused-why")
                    .flex()
                    .items_center()
                    .gap(px(4.))
                    .cursor_pointer()
                    .text_size(px(13.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(ink3)
                    .child(ui::icon(if open { "chevron-down" } else { "chevron-right" }, 12., ink3))
                    .child(t(cx, "huginn.why"))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        let e = this.ui.open.entry(key.clone()).or_insert((false, false));
                        e.1 = !e.1;
                        cx.notify();
                    })),
            )
            .when(open, |d| {
                d.child(div().mt(px(6.)).flex().flex_col().font_family(ui::MONO).text_size(px(12.)).children(issues.iter().map(|i| {
                    let element = i.element.as_ref().map(|e| format!(" ({e})")).unwrap_or_default();
                    div().child(format!("{file}: {}{element}: {}", i.code, i.message))
                })))
            })
            .into_any_element()
    }
}

impl Render for HuginnView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.global::<Theme>();
        let pal = Pal::new(theme);
        let locale = cx.global::<I18n>().locale;
        let wait = |key: &str, cx: &mut Context<Self>| {
            div().text_size(px(13.)).text_color(pal.c("ink-4")).child(t(cx, key)).into_any_element()
        };
        let spec = match &self.load {
            Load::Loading => return wait("huginn.viewLoading", cx),
            Load::Unreadable => return wait("huginn.viewUnreadable", cx),
            Load::Refused(issues) => {
                let issues = issues.clone();
                return self.refused(&issues, &pal, cx);
            }
            Load::Ready(spec) => spec.clone(),
        };
        let target = self.target();
        let entity = cx.entity_id();
        let (root, publish, seen, subs) = {
            let mut b = B {
                spec: &spec,
                state: &self.state,
                ui: &mut self.ui,
                pal: &pal,
                locale,
                window,
                cx,
                entity,
                file: &self.attachment.name,
                files: &self.message.attachments,
                target,
                given: &self.given,
                publish: Vec::new(),
                seen: HashSet::new(),
                form: None,
                subs: Vec::new(),
            };
            let root = b.element(&spec.root, Scope { item: None, index: None });
            (root, b.publish, b.seen, b.subs)
        };
        self.subs.extend(subs);
        // Fields that did not render this frame leave their Form.
        for (form, ask) in self.ui.asks.iter_mut() {
            ask.fields.retain(|field, _| seen.contains(&format!("{form}/{field}")));
        }
        let mut changed = false;
        for (path, value) in publish {
            if expr::get(&self.state, &path) != Some(&value) {
                expr::set(&mut self.state, &path, value);
                changed = true;
            }
        }
        if changed {
            cx.notify();
        }
        div()
            .mt(px(8.))
            .max_w(px(760.))
            .p(px(14.))
            .border_1()
            .border_color(pal.c("line"))
            .rounded(px(10.))
            .bg(pal.c("surface"))
            .text_color(pal.c("ink"))
            .font_family(ui::SANS)
            .text_size(px(15.))
            .line_height(relative(1.45))
            .flex()
            .flex_col()
            .children(root)
            .into_any_element()
    }
}
