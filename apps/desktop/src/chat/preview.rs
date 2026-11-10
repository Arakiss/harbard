//! A dev window for the message layer: `harbard-app --chat-preview` opens
//! the default project channel's stream with a composer under it, instead
//! of the workspace. Dev switches (environment): `HARBARD_PREVIEW_CHANNEL`,
//! `_SKIP=n`, `_ONLY=<id prefix>`, `_ALL=1`, `_FLAT=1`, `_NOATT=1`, `_HEAD=1`.

use super::{ComposeTarget, Composer, MessageOpts, ThreadOpener};
use crate::desk::Desk;
use crate::i18n::tv;
use crate::model::Message;
use crate::theme::Theme;
use gpui::{
    App, AppContext, Bounds, Context, Entity, IntoElement, ParentElement, Render, Styled, TitlebarOptions, Window,
    WindowBounds, WindowOptions, div, point, px, size,
};

pub fn wanted() -> bool {
    std::env::args().any(|a| a == "--chat-preview")
}

pub fn open(cx: &mut App) {
    let bounds = Bounds::centered(None, size(px(1440.), px(900.)), cx);
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        titlebar: Some(TitlebarOptions {
            title: Some("chat preview".into()),
            appears_transparent: true,
            traffic_light_position: Some(point(px(14.), px(18.))),
        }),
        ..Default::default()
    };
    gpui_kit::open_window(options, cx, |window, cx| cx.new(|cx| Preview::new(window, cx))).expect("open the chat preview");
}

struct Preview {
    channel: Option<String>,
    composer: Entity<Composer>,
}

/// The default project channel: the first that is not a room nor archived,
/// preferring one with messages (`HARBARD_PREVIEW_CHANNEL` picks another).
fn default_channel(cx: &App) -> Option<String> {
    if let Ok(id) = std::env::var("HARBARD_PREVIEW_CHANNEL") {
        return Some(id);
    }
    let desk = Desk::get(cx).read(cx);
    let view = desk.view.as_ref()?;
    let open = || view.channels.iter().filter(|c| !c.is_room() && !c.archived);
    let said = |id: &str| view.messages.iter().any(|m| m.channels.iter().any(|c| c == id));
    open().find(|c| said(&c.id)).or_else(|| open().next()).map(|c| c.id.clone())
}

impl Preview {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        cx.observe(&Desk::get(cx), |this: &mut Preview, _, cx| {
            this.sync(cx);
            cx.notify();
        })
        .detach();
        cx.observe_global::<Theme>(|_, cx| cx.notify()).detach();
        let channel = default_channel(cx);
        let target = ComposeTarget::Channel(channel.clone().unwrap_or_default());
        let composer = cx.new(|cx| Composer::new(target, window, cx));
        let mut this = Preview { channel, composer };
        this.sync(cx);
        this
    }

    /// Point the composer at the default channel once the read names it.
    fn sync(&mut self, cx: &mut Context<Self>) {
        let channel = default_channel(cx);
        if let Some(id) = &channel {
            let placeholder = tv(cx, "home.writeInChannel", &[("channel", id)]);
            let target = ComposeTarget::Channel(id.clone());
            self.composer.update(cx, |c, cx| c.set_target(target, placeholder, cx));
        }
        self.channel = channel;
    }
}

impl Render for Preview {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.global::<Theme>().clone();
        let desk = Desk::get(cx).read(cx);
        let view = desk.view.clone();
        let body = match (&view, &self.channel) {
            (Some(view), Some(id)) => {
                let messages: Vec<&Message> = view
                    .messages
                    .iter()
                    .filter(|m| {
                        // `HARBARD_PREVIEW_ALL=1` keeps replies and every channel too.
                        std::env::var_os("HARBARD_PREVIEW_ALL").is_some()
                            || (m.channels.iter().any(|c| c == id) && m.reply_to.is_none() && m.structured.is_none() && m.from != "peer")
                    })
                    .collect();
                // `HARBARD_PREVIEW_SKIP=n` drops the last n, to look at older ones.
                let skip = std::env::var("HARBARD_PREVIEW_SKIP").ok().and_then(|n| n.parse::<usize>().ok()).unwrap_or(0);
                let mut messages = messages[..messages.len().saturating_sub(skip)].to_vec();
                // `HARBARD_PREVIEW_ONLY=<id prefix>` keeps the messages whose id starts so.
                if let Ok(only) = std::env::var("HARBARD_PREVIEW_ONLY") {
                    messages.retain(|m| m.id.starts_with(&only));
                }
                // `HARBARD_PREVIEW_NOATT=1` draws copies without attachments, to see the text above them.
                let bare: Vec<Message> = if std::env::var_os("HARBARD_PREVIEW_NOATT").is_some() {
                    messages.iter().map(|m| Message { attachments: Vec::new(), ..(*m).clone() }).collect()
                } else {
                    Vec::new()
                };
                if !bare.is_empty() {
                    messages = bare.iter().collect();
                }
                let opts = MessageOpts {
                    fold: true,
                    // `HARBARD_PREVIEW_FLAT=1` shows internal updates unfolded.
                    fold_internal: std::env::var_os("HARBARD_PREVIEW_FLAT").is_none(),
                    on_thread: Some(ThreadOpener::new(|id, _, _| eprintln!("chat preview: open thread {id}"))),
                    key: Some(format!("preview-{id}").into()),
                    empty: Some(tv(cx, "home.nobodyWroteYet", &[("channel", id)])),
                    ..Default::default()
                };
                if std::env::var_os("HARBARD_PREVIEW_HEAD").is_some() {
                    // `HARBARD_PREVIEW_HEAD=1`: a head row and the near-top callback, as rooms use them.
                    let head = div().px(px(20.)).py(px(6.)).text_color(theme.c("ink-4")).child("head row").into_any_element();
                    let near: super::stream::NearTop = std::rc::Rc::new(|_, _| eprintln!("chat preview: near the top"));
                    super::stream::stream_with(&messages, view, &opts, Some(head), Some(near), cx)
                } else {
                    super::stream(&messages, view, &opts, cx)
                }
            }
            _ => crate::ui::empty(crate::i18n::t(cx, "messages.loading"), &theme).into_any_element(),
        };
        div()
            .size_full()
            .flex()
            .flex_col()
            .pt(px(40.))
            .bg(theme.c("surface"))
            .text_color(theme.c("ink"))
            .font_family(crate::ui::SANS)
            .text_size(px(crate::ui::size::BODY))
            .child(div().flex_1().min_h_0().flex().flex_col().child(body))
            .child(self.composer.clone())
    }
}
