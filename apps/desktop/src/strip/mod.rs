//! The Strip window, "The factory" (`/pulse`, `components/strip/*`): what
//! needs the operator on the left, the agents of the last two hours as
//! robots on a shelf in the middle, the clock and the day's deliveries on the
//! right. It reads the whole hub (`ws=all`); a tap opens the main window
//! there. Nothing here writes to serve.
//!
//! Drawn on a fixed 2560×720 stage scaled to the window ([`model::strip_fit`]),
//! so the wide display and an ordinary window paint the same composition.

pub mod model;
pub mod status;

use crate::desk::Desk;
use crate::i18n::{I18n, t, tv};
use crate::model::{Agent, Card, WorkspaceView};
use crate::native::{self, StripMode, appkit};
use crate::nav::Route;
use crate::theme::Theme;
use crate::ui::{self, SANS};
use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, App, AppContext, Bounds, Context, FocusHandle, FontWeight, Hsla, InteractiveElement, IntoElement,
    ParentElement, Pixels, Render, Rgba, SharedString, StatefulInteractiveElement, Styled, TitlebarOptions, Window,
    WindowBounds, WindowControlArea, WindowOptions, div, img, point, px, size, svg,
};
use model::{STAGE_H, STAGE_W, Tone};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Open the Strip window in its mode and remember it.
pub fn open(mode: StripMode, focus: bool, cx: &mut App) {
    let title: SharedString = native::tr(cx, "stripTitle", &[]).into();
    let wide = native::wide_screen().filter(|_| mode == StripMode::Wide);
    let options = match wide {
        Some(screen) => WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds {
                origin: point(px(0.), px(0.)),
                size: size(px(screen.w as f32), px(screen.h as f32)),
            })),
            titlebar: None,
            focus,
            show: true,
            is_movable: false,
            is_resizable: false,
            is_minimizable: false,
            app_id: Some("com.harbard.desktop".into()),
            ..Default::default()
        },
        None => {
            let area = cx
                .primary_display()
                .map(|d| d.visible_bounds().size)
                .unwrap_or(size(px(1440.), px(900.)));
            let (w, h) = model::fallback_size(f32::from(area.width), f32::from(area.height));
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds::centered(None, size(px(w), px(h)), cx))),
                titlebar: Some(TitlebarOptions {
                    title: Some(title.clone()),
                    appears_transparent: false,
                    traffic_light_position: None,
                }),
                window_min_size: Some(size(px(model::FALLBACK_MIN_W), px(model::FALLBACK_MIN_W * 9. / 32.))),
                focus,
                show: true,
                app_id: Some("com.harbard.desktop".into()),
                ..Default::default()
            }
        }
    };
    let opened = gpui_kit::open_window(options, cx, |window, cx| cx.new(|cx| Strip::new(window, cx)));
    let Ok((handle, _)) = opened else {
        eprintln!("harbard: {}", native::tr(cx, "couldNotOpenStrip", &[]));
        return;
    };
    cx.global_mut::<native::Native>().strip = Some(handle);
    let _ = handle.update(cx, |_, window, cx| {
        window.set_window_title(&title);
        if let Some(screen) = wide {
            appkit::make_borderless_at(window, screen.frame());
        }
        window.on_window_should_close(cx, |window, _| {
            appkit::hide(window);
            false
        });
    });
}

pub struct Strip {
    all: Option<Arc<WorkspaceView>>,
    failed: bool,
    reading: bool,
    again: bool,
    read_at: Option<Instant>,
    today_open: bool,
    today_seq: u64,
    focus: FocusHandle,
}

/// Reads of the whole hub at most this often.
const READ_EVERY: Duration = Duration::from_millis(1000);

impl Strip {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let desk = Desk::get(cx);
        cx.observe(&desk, |this: &mut Strip, _, cx| this.refresh(cx)).detach();
        cx.observe_global::<Theme>(|_, cx| cx.notify()).detach();
        cx.observe_global::<I18n>(|_, cx| cx.notify()).detach();
        // Timers tick by the second; the clock by the minute.
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(Duration::from_secs(1)).await;
                if this.update(cx, |_, cx| cx.notify()).is_err() {
                    break;
                }
            }
        })
        .detach();
        let focus = cx.focus_handle();
        window.focus(&focus, cx);
        let mut strip = Strip {
            all: None,
            failed: false,
            reading: false,
            again: false,
            read_at: None,
            today_open: false,
            today_seq: 0,
            focus,
        };
        strip.refresh(cx);
        strip
    }

    /// Read the whole hub in the background; one read at a time, at most one
    /// a second, the latest change always read.
    fn refresh(&mut self, cx: &mut Context<Self>) {
        if self.reading {
            self.again = true;
            return;
        }
        if let Some(at) = self.read_at
            && at.elapsed() < READ_EVERY
        {
            if !self.again {
                self.again = true;
                let wait = READ_EVERY - at.elapsed();
                cx.spawn(async move |this, cx| {
                    cx.background_executor().timer(wait).await;
                    let _ = this.update(cx, |this, cx| {
                        this.again = false;
                        this.refresh(cx);
                    });
                })
                .detach();
            }
            return;
        }
        self.reading = true;
        self.read_at = Some(Instant::now());
        let desk = Desk::get(cx).read(cx);
        let store = desk.hub.store().clone();
        let events = desk.hub.events();
        let task = cx.background_spawn(async move {
            crate::hub::read(&store, &events, Some("all".into()), false)
                .and_then(|v| serde_json::from_value::<WorkspaceView>(v).map_err(|e| e.to_string()))
        });
        cx.spawn(async move |this, cx| {
            let read = task.await;
            let _ = this.update(cx, |this, cx| {
                match read {
                    Ok(v) => {
                        if this.all.as_deref() != Some(&v) {
                            this.all = Some(Arc::new(v));
                        }
                        this.failed = false;
                    }
                    Err(_) => this.failed = true,
                }
                this.reading = false;
                if std::mem::take(&mut this.again) {
                    this.refresh(cx);
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn toggle_today(&mut self, cx: &mut Context<Self>) {
        self.today_open = !self.today_open;
        self.today_seq += 1;
        if self.today_open {
            let seq = self.today_seq;
            cx.spawn(async move |this, cx| {
                cx.background_executor()
                    .timer(Duration::from_millis(model::TODAY_OPEN_MS))
                    .await;
                let _ = this.update(cx, |this, cx| {
                    if this.today_seq == seq {
                        this.today_open = false;
                        cx.notify();
                    }
                });
            })
            .detach();
        }
        cx.notify();
    }
}

/// Where a tap goes: the main window at `route`, in `workspace` when the
/// item belongs to another one.
fn go(route: Route, workspace: Option<String>, cx: &mut App) {
    let desk = Desk::get(cx);
    let current = desk.read(cx).workspace.clone();
    if let Some(ws) = workspace
        && current.as_deref() != Some(ws.as_str())
    {
        desk.update(cx, |d, cx| d.switch_workspace(Some(ws), cx));
    }
    native::open_main(Some(route), cx);
}

// ---------------------------------------------------------------------------
// Colours: the strip's mixes of the theme's tokens (strip.css)
// ---------------------------------------------------------------------------

/// `color-mix(in srgb, a p, b)`.
fn mix(a: Hsla, b: Hsla, p: f32) -> Hsla {
    let (a, b) = (Rgba::from(a), Rgba::from(b));
    let q = 1. - p;
    Hsla::from(Rgba {
        r: a.r * p + b.r * q,
        g: a.g * p + b.g * q,
        b: a.b * p + b.b * q,
        a: a.a * p + b.a * q,
    })
}

fn alpha(c: Hsla, a: f32) -> Hsla {
    Hsla { a: c.a * a, ..c }
}

struct Palette {
    frame: Hsla,
    page: Hsla,
    panel: Hsla,
    line: Hsla,
    ink: Hsla,
    clock_ink: Hsla,
    front: Hsla,
    amber_card: Hsla,
    amber_pill: Hsla,
    lilac_pill: Hsla,
    muted_card: Hsla,
    green_soft: Hsla,
    shadow: Hsla,
}

fn tok(theme: &Theme, name: &str, fallback: &str) -> Hsla {
    if theme.raw(name).is_some() {
        theme.c(name)
    } else {
        theme.c(fallback)
    }
}

impl Palette {
    fn of(th: &Theme) -> Palette {
        let c = |n: &str| th.c(n);
        let page = mix(c("tint"), mix(c("amber-bg"), c("red-bg"), 0.62), 0.60);
        let panel = mix(c("surface"), page, 0.55);
        Palette {
            frame: c("frame"),
            page,
            panel,
            line: mix(c("line"), page, 0.60),
            ink: mix(c("ink"), c("accent-text"), 0.85),
            clock_ink: mix(c("ink"), c("accent-text"), 0.55),
            front: mix(c("surface"), mix(c("amber-bg"), c("red-bg"), 0.55), 0.60),
            amber_card: mix(c("amber-bg"), c("amber"), 0.88),
            amber_pill: mix(c("amber"), c("amber-bg"), 0.46),
            lilac_pill: mix(c("accent-soft"), c("badge"), 0.55),
            muted_card: mix(page, c("accent-soft"), 0.70),
            green_soft: mix(c("green-bg"), panel, 0.45),
            shadow: alpha(c("ink"), 0.14),
        }
    }
}

// ---------------------------------------------------------------------------
// Drawing
// ---------------------------------------------------------------------------

/// Stage pixels to window pixels.
#[derive(Clone, Copy)]
struct K(f32);

impl K {
    fn px(self, v: f32) -> Pixels {
        px(v * self.0)
    }
}

fn label(text: impl Into<SharedString>, k: K, size: f32, weight: FontWeight, color: Hsla) -> gpui::Div {
    div()
        .font_family(SANS)
        .text_size(k.px(size))
        .line_height(k.px(size * 1.3))
        .font_weight(weight)
        .text_color(color)
        .child(text.into())
}

impl Render for Strip {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.global::<Theme>().clone();
        let p = Palette::of(&theme);
        let vp = window.viewport_size();
        let (k, left, top) = model::strip_fit(f32::from(vp.width), f32::from(vp.height));
        let k = K(k);
        let desk = Desk::get(cx).read(cx);
        let failed = self.failed || desk.error.is_some();
        let logo = native::logo::logo_state(desk);
        let now = self
            .all
            .as_ref()
            .map(|v| v.now_ms)
            .filter(|n| *n > 0)
            .map_or_else(|| desk.now_ms(), |_| desk.now_ms());
        let os_dark = matches!(
            cx.window_appearance(),
            gpui::WindowAppearance::Dark | gpui::WindowAppearance::VibrantDark
        );
        let dim = model::dimmed(
            harbard_core::clock::now_ms(),
            os_dark,
            self.all.as_ref().is_none_or(|v| v.night_dim),
        );
        let all = self.all.clone();

        let body: Vec<AnyElement> = match all.as_deref() {
            Some(view) => vec![
                self.needs(view, now, k, &p, &theme, cx),
                self.shelf(view, now, k, &p, &theme, cx),
                self.side(view, now, k, &p, &theme, cx),
            ],
            None => vec![
                panel(k, &p).w(k.px(524.)).into_any_element(),
                panel(k, &p)
                    .w(k.px(1500.))
                    .child(
                        div()
                            .absolute()
                            .left(k.px(7.))
                            .right(k.px(7.))
                            .bottom(k.px(5.))
                            .h(k.px(132.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(label(
                                t(
                                    cx,
                                    if failed {
                                        "strip.notResponding"
                                    } else {
                                        "strip.connecting"
                                    },
                                ),
                                k,
                                30.,
                                FontWeight::BOLD,
                                theme.c("ink-3"),
                            )),
                    )
                    .into_any_element(),
                panel(k, &p).w(k.px(464.)).into_any_element(),
            ],
        };

        div()
            .id("strip")
            .track_focus(&self.focus)
            .on_key_down(cx.listener(|_, ev: &gpui::KeyDownEvent, _, cx| {
                let m = &ev.keystroke.modifiers;
                if ev.keystroke.key == "escape" && !m.platform && !m.control && !m.alt && !m.shift {
                    native::close_strip(cx);
                }
            }))
            .size_full()
            .relative()
            .bg(p.frame)
            .font_family(SANS)
            .child(
                div()
                    .absolute()
                    .left(px(left))
                    .top(px(top))
                    .w(k.px(STAGE_W))
                    .h(k.px(STAGE_H))
                    .overflow_hidden()
                    .bg(p.frame)
                    .child(header(logo, failed, k, &theme, cx))
                    .child(
                        div()
                            .absolute()
                            .top(k.px(96.))
                            .left_0()
                            .right_0()
                            .bottom_0()
                            .flex()
                            .gap(k.px(18.))
                            .pt(k.px(28.))
                            .px(k.px(18.))
                            .pb(k.px(40.))
                            .rounded_tl(k.px(28.))
                            .rounded_tr(k.px(28.))
                            .bg(p.page)
                            .children(body),
                    )
                    .when(dim, |d| {
                        d.child(div().absolute().inset_0().bg(alpha(theme.c("st-term"), 0.45)))
                    }),
            )
    }
}

fn panel(k: K, p: &Palette) -> gpui::Div {
    div()
        .relative()
        .flex_none()
        .h_full()
        .overflow_hidden()
        .border(k.px(2.))
        .border_color(p.line)
        .rounded(k.px(26.))
        .bg(p.panel)
        .shadow(vec![gpui::BoxShadow {
            color: p.shadow,
            offset: point(px(0.), k.px(3.)),
            blur_radius: k.px(12.),
            spread_radius: k.px(-6.),
            inset: false,
        }])
}

/// The band: the living mark, the name, the view's title.
fn header(logo: native::dock::LogoState, failed: bool, k: K, theme: &Theme, cx: &App) -> AnyElement {
    use native::dock::LogoState as L;
    let ink = theme.c("rail-ink");
    let (dot, hollow) = match logo {
        L::Blocked => (tok(theme, "mark-dot-red", "red"), false),
        L::Needs => (tok(theme, "mark-dot-amber", "amber"), false),
        L::Connecting | L::Disconnected => (tok(theme, "mark-dot-grey", "grey"), true),
        _ => (tok(theme, "mark-dot", "green"), false),
    };
    let m = 58.;
    let r = 7. / 64. * m;
    let mark = div()
        .relative()
        .flex_none()
        .size(k.px(m))
        .child(
            svg()
                .path("strip/mark.svg")
                .size(k.px(m))
                .text_color(if logo == L::Disconnected { alpha(ink, 0.45) } else { ink }),
        )
        .child(
            div()
                .absolute()
                .left(k.px(54. / 64. * m - r))
                .top(k.px(13. / 64. * m - r))
                .size(k.px(r * 2.))
                .rounded_full()
                .when(!hollow, |d| d.bg(dot))
                .when(hollow, |d| d.border(k.px(2.2 * m / 64.)).border_color(dot)),
        );
    div()
        .absolute()
        .top_0()
        .left_0()
        .right_0()
        .h(k.px(96.))
        .flex()
        .items_center()
        .px(k.px(30.))
        .text_color(ink)
        .window_control_area(WindowControlArea::Drag)
        .child(mark)
        .child(
            div()
                .ml(k.px(26.))
                .text_size(k.px(48.))
                .line_height(k.px(48.))
                .font_weight(FontWeight::BOLD)
                .child("harbard"),
        )
        .child(
            div()
                .w(k.px(2.))
                .h(k.px(56.))
                .ml(k.px(31.))
                .mr(k.px(27.))
                .bg(alpha(ink, 0.28)),
        )
        .child(
            div()
                .flex()
                .items_center()
                .gap(k.px(20.))
                .h(k.px(66.))
                .pl(k.px(24.))
                .pr(k.px(26.))
                .rounded(k.px(14.))
                .bg(mix(theme.c("rail-tile"), theme.c("ws-tile"), 0.55))
                .text_size(k.px(34.))
                .line_height(k.px(34.))
                .font_weight(FontWeight::BOLD)
                .child(ui::icon("factory", 42. * k.0, ink))
                .child(t(cx, "strip.factory")),
        )
        .when(failed, |d| {
            d.child(
                div()
                    .ml_auto()
                    .text_size(k.px(22.))
                    .text_color(theme.c("rail-text"))
                    .child(t(cx, "strip.offlineNote")),
            )
        })
        .into_any_element()
}

impl Strip {
    fn needs(
        &self,
        view: &WorkspaceView,
        now: u64,
        k: K,
        p: &Palette,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let (shown, more) = model::needs(view);
        let three = shown.len() == 3;
        let mut col = div()
            .flex()
            .flex_col()
            .gap(k.px(if three { 16. } else { 22. }))
            .pt(k.px(34.))
            .px(k.px(14.))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(k.px(18.))
                    .h(k.px(64.))
                    .ml(k.px(13.))
                    .mb(k.px(if three { 4. } else { 12. }))
                    .child(
                        div()
                            .size(k.px(64.))
                            .rounded(k.px(16.))
                            .bg(theme.c("red-bg"))
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(ui::icon("bell", 36. * k.0, theme.c("red"))),
                    )
                    .child(
                        div()
                            .text_size(k.px(50.))
                            .line_height(k.px(50.))
                            .font_weight(FontWeight::BLACK)
                            .text_color(p.ink)
                            .child(t(cx, "strip.needsYou")),
                    ),
            );
        if shown.is_empty() {
            col = col.child(
                div()
                    .px(k.px(14.))
                    .text_size(k.px(26.))
                    .text_color(theme.c("ink-3"))
                    .child(t(cx, "strip.nothingNeedsYou")),
            );
        }
        for (i, card) in shown.iter().enumerate() {
            col = col.child(if i == 0 {
                big_card(card, view, now, three, k, p, theme, cx)
            } else {
                small_card(card, view, now, three, i, k, p, theme, cx)
            });
        }
        if more > 0 {
            col = col.child(
                div()
                    .id("needs-more")
                    .cursor_pointer()
                    .pl(k.px(16.))
                    .text_size(k.px(23.))
                    .font_weight(FontWeight::BOLD)
                    .text_color(theme.c("link"))
                    .on_click(|_, _, cx| go(Route::activity(), None, cx))
                    .child(tv(cx, "strip.moreCount", &[("count", &more.to_string())])),
            );
        }
        panel(k, p).w(k.px(524.)).child(col).into_any_element()
    }

    fn shelf(
        &self,
        view: &WorkspaceView,
        now: u64,
        k: K,
        p: &Palette,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let s = model::shelf(view, now);
        let (slot, bot) = model::shelf_geometry(s.shown.len(), s.more > 0);
        let name_px = s
            .shown
            .iter()
            .map(|a| model::name_size(&a.name, slot))
            .fold(model::NAME_MAX, f32::min);
        let plate_h = 132.;
        let feet = plate_h + 10.;
        let backdrop = if theme.dark {
            "strip/factory-dark-2x.webp"
        } else {
            "strip/factory-light-2x.webp"
        };
        let mut bots = div()
            .absolute()
            .top_0()
            .left(k.px(28.))
            .right(k.px(28.))
            .bottom(k.px(5.))
            .flex()
            .justify_center();
        for agent in &s.shown {
            bots = bots.child(shelf_bot(
                agent, view, now, s.empty, slot, bot, name_px, feet, plate_h, k, p, theme, cx,
            ));
        }
        if s.more > 0 {
            let more = s.more;
            bots = bots.child(
                div()
                    .id("shelf-more")
                    .relative()
                    .flex_none()
                    .w(k.px(104.))
                    .h_full()
                    .cursor_pointer()
                    .on_click(|_, _, cx| go(Route::direct(None), None, cx))
                    .child(
                        div()
                            .absolute()
                            .left(k.px(8.))
                            .bottom(k.px(feet + 60.))
                            .size(k.px(88.))
                            .rounded_full()
                            .border(k.px(2.))
                            .border_color(p.line)
                            .bg(theme.c("accent-soft"))
                            .text_color(theme.c("accent-text"))
                            .flex()
                            .items_center()
                            .justify_center()
                            .text_size(k.px(32.))
                            .font_weight(FontWeight::BLACK)
                            .child(format!("+{more}")),
                    )
                    .child(
                        div()
                            .absolute()
                            .left_0()
                            .right_0()
                            .bottom_0()
                            .h(k.px(plate_h))
                            .pt(k.px(13.))
                            .flex()
                            .justify_center()
                            .text_size(k.px(30.))
                            .font_weight(FontWeight::BLACK)
                            .text_color(p.ink)
                            .child(t(cx, "strip.moreLabel")),
                    ),
            );
        }
        let front = div()
            .absolute()
            .left(k.px(7.))
            .right(k.px(7.))
            .bottom(k.px(5.))
            .h(k.px(plate_h))
            .rounded(k.px(24.))
            .border(k.px(1.5))
            .border_color(mix(p.line, theme.c("surface"), 0.7))
            .bg(mix(p.front, theme.c("surface"), 0.85))
            .shadow(vec![gpui::BoxShadow {
                color: p.shadow,
                offset: point(px(0.), k.px(12.)),
                blur_radius: k.px(22.),
                spread_radius: k.px(-12.),
                inset: false,
            }]);
        let today = model::today(view, now);
        panel(k, p)
            .w(k.px(1500.))
            .child(
                div()
                    .absolute()
                    .top_0()
                    .left_0()
                    .right_0()
                    .h(k.px(459.))
                    .flex()
                    .justify_center()
                    .child(img(backdrop).w(k.px(1616.)).h(k.px(459.)).flex_none()),
            )
            .child(front)
            .child(bots)
            .when(s.empty, |d| {
                d.child(
                    div()
                        .absolute()
                        .left(k.px(7.))
                        .right(k.px(7.))
                        .bottom(k.px(5.))
                        .h(k.px(plate_h))
                        .flex()
                        .items_center()
                        .justify_center()
                        .text_size(k.px(34.))
                        .font_weight(FontWeight::BOLD)
                        .text_color(theme.c("ink-2"))
                        .child(t(cx, "strip.nobodyWorking")),
                )
            })
            .when(self.today_open, |d| {
                d.child(self.today_sheet(view, &today, k, p, theme, cx))
            })
            .into_any_element()
    }

    fn today_sheet(
        &self,
        view: &WorkspaceView,
        today: &model::Today<'_>,
        k: K,
        p: &Palette,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let shown: Vec<_> = today.items.iter().take(10).collect();
        let rest = today.items.len() - shown.len();
        let mut sheet = div()
            .id("today-sheet")
            .absolute()
            .inset_0()
            .py(k.px(30.))
            .px(k.px(44.))
            .bg(p.panel)
            .rounded(k.px(24.))
            .on_click(cx.listener(|this, _, _, cx| this.toggle_today(cx)))
            .child(
                div()
                    .text_size(k.px(56.))
                    .line_height(k.px(62.))
                    .font_weight(FontWeight::BLACK)
                    .text_color(p.ink)
                    .child(t(cx, "strip.today")),
            );
        if shown.is_empty() {
            sheet = sheet.child(div().mt(k.px(24.)).child(label(
                t(cx, "strip.nothingDeliveredToday"),
                k,
                30.,
                FontWeight::BOLD,
                theme.c("ink-3"),
            )));
        } else {
            // Two columns of five rows, filled down then across.
            let row = |m: &crate::model::Message| {
                let who = view
                    .agents
                    .iter()
                    .find(|a| a.key == m.agent)
                    .map_or(m.author.clone(), |a| a.name.clone());
                div()
                    .flex()
                    .items_center()
                    .gap(k.px(8.))
                    .h(k.px(82.))
                    .border_b(k.px(1.5))
                    .border_color(p.line)
                    .child(
                        div().w(k.px(34.)).flex_none().child(
                            div()
                                .size(k.px(18.))
                                .rounded_full()
                                .bg(theme.c("green"))
                                .border(k.px(5.))
                                .border_color(theme.c("green-bg")),
                        ),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .flex_1()
                            .min_w_0()
                            .child(label(
                                format!("{} · {who}", crate::ui::format::hhmm(m.ts_ms)),
                                k,
                                20.,
                                FontWeight::NORMAL,
                                theme.c("ink-3"),
                            ))
                            .child(
                                label(model::sentence(&m.text), k, 26., FontWeight::NORMAL, p.ink)
                                    .whitespace_nowrap()
                                    .overflow_hidden()
                                    .text_ellipsis(),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_none()
                            .gap(k.px(10.))
                            .child(
                                label(t(cx, "strip.delivered"), k, 20., FontWeight::BOLD, theme.c("green-ink"))
                                    .px(k.px(13.))
                                    .py(k.px(4.))
                                    .rounded(k.px(10.))
                                    .bg(theme.c("green-bg")),
                            )
                            .when(m.verified.is_some(), |d| {
                                d.child(
                                    label(
                                        t(cx, "strip.verifiedChip"),
                                        k,
                                        20.,
                                        FontWeight::NORMAL,
                                        theme.c("chip-ink"),
                                    )
                                    .px(k.px(13.))
                                    .py(k.px(4.))
                                    .rounded(k.px(10.))
                                    .bg(theme.c("chip")),
                                )
                            }),
                    )
            };
            let col = |items: &[&&crate::model::Message]| {
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .children(items.iter().map(|m| row(m)))
            };
            let (a, b) = shown.split_at(shown.len().min(5));
            sheet = sheet.child(div().mt(k.px(14.)).flex().gap(k.px(56.)).child(col(a)).child(col(b)));
        }
        if rest > 0 {
            sheet = sheet.child(
                div()
                    .mt(k.px(12.))
                    .text_size(k.px(22.))
                    .text_color(theme.c("ink-3"))
                    .child(tv(cx, "strip.restInActivity", &[("count", &rest.to_string())])),
            );
        }
        sheet.into_any_element()
    }

    fn side(
        &self,
        view: &WorkspaceView,
        now: u64,
        k: K,
        p: &Palette,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let spanish = cx.global::<I18n>().locale == crate::i18n::Locale::Es;
        let (time, date) = model::clock(harbard_core::clock::now_ms(), spanish);
        let day = model::today(view, now);
        let line = model::today_line(day.items.len(), day.verified, cx);
        panel(k, p)
            .w(k.px(464.))
            .flex()
            .flex_col()
            .gap(k.px(41.))
            .pt(k.px(86.))
            .pl(k.px(19.))
            .pr(k.px(18.))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .items_center()
                    .justify_center()
                    .h(k.px(232.))
                    .rounded(k.px(26.))
                    .bg(theme.c("accent-soft"))
                    .text_color(p.clock_ink)
                    .child(
                        div()
                            .text_size(k.px(126.))
                            .line_height(k.px(121.))
                            .font_weight(FontWeight::BLACK)
                            .child(time),
                    )
                    .child(
                        div()
                            .mt(k.px(10.))
                            .text_size(k.px(36.))
                            .line_height(k.px(40.))
                            .child(date),
                    ),
            )
            .child(
                div()
                    .id("today")
                    .cursor_pointer()
                    .flex()
                    .items_center()
                    .gap(k.px(14.))
                    .h(k.px(63.))
                    .px(k.px(14.))
                    .rounded(k.px(16.))
                    .bg(p.green_soft)
                    .text_color(p.ink)
                    .text_size(k.px(23.))
                    .whitespace_nowrap()
                    .on_click(cx.listener(|this, _, _, cx| this.toggle_today(cx)))
                    .child(ui::icon("package", 34. * k.0, theme.c("green-ink")))
                    .child(line),
            )
            .into_any_element()
    }
}

#[allow(clippy::too_many_arguments)]
fn big_card(
    card: &Card,
    view: &WorkspaceView,
    now: u64,
    three: bool,
    k: K,
    p: &Palette,
    theme: &Theme,
    cx: &App,
) -> AnyElement {
    let c = model::card_view(card, view, now, cx);
    let (bg, edge) = match c.tone {
        Tone::Warn => (p.amber_card, theme.c("warn-line")),
        Tone::Alert => (theme.c("red-bg"), theme.c("red-line")),
        Tone::Quiet => (p.muted_card, p.line),
    };
    let route = Route::Activity {
        tab: "needs".into(),
        sel: Some(format!("card:{}", card.id)),
    };
    let ws = card.workspace.clone();
    div()
        .relative()
        .flex_none()
        .h(k.px(if three { 194. } else { 232. }))
        .pt(k.px(20.))
        .px(k.px(22.))
        .rounded(k.px(20.))
        .border(k.px(2.))
        .border_color(edge)
        .bg(bg)
        .text_color(p.ink)
        .child(card_top(&c, k, p, theme, cx, 27., 26., 22.))
        .when(!c.text.is_empty(), |d| {
            d.child(
                div()
                    .mt(k.px(20.))
                    .text_size(k.px(28.))
                    .line_height(k.px(28. * 1.38))
                    .line_clamp(if three { 1 } else { 2 })
                    .child(c.text.clone()),
            )
        })
        .child(
            div()
                .id(SharedString::from(format!("reply-{}", card.id)))
                .absolute()
                .right(k.px(20.))
                .bottom(k.px(21.))
                .flex()
                .items_center()
                .h(k.px(52.))
                .px(k.px(21.))
                .rounded(k.px(12.))
                .bg(theme.c("accent-deep"))
                .text_color(theme.c("accent-ink"))
                .text_size(k.px(24.))
                .font_weight(FontWeight::BOLD)
                .cursor_pointer()
                .on_click(move |_, _, cx| go(route.clone(), ws.clone(), cx))
                .child(t(cx, if c.reply { "strip.reply" } else { "strip.open" })),
        )
        .into_any_element()
}

#[allow(clippy::too_many_arguments)]
fn small_card(
    card: &Card,
    view: &WorkspaceView,
    now: u64,
    three: bool,
    index: usize,
    k: K,
    p: &Palette,
    theme: &Theme,
    cx: &App,
) -> AnyElement {
    let c = model::card_view(card, view, now, cx);
    let (bg, edge) = match c.tone {
        Tone::Warn => (p.amber_card, theme.c("warn-line")),
        _ => (p.muted_card, p.line),
    };
    let route = Route::Activity {
        tab: "needs".into(),
        sel: Some(format!("card:{}", card.id)),
    };
    let ws = card.workspace.clone();
    div()
        .id(SharedString::from(format!("small-{}", card.id)))
        .flex()
        .flex_none()
        .items_center()
        .h(k.px(if three { 70. } else { 81. }))
        .when(index > 1, |d| d.mt(k.px(-6.)))
        .px(k.px(18.))
        .rounded(k.px(18.))
        .border(k.px(2.))
        .border_color(edge)
        .bg(bg)
        .text_color(p.ink)
        .cursor_pointer()
        .on_click(move |_, _, cx| go(route.clone(), ws.clone(), cx))
        .child(card_top(&c, k, p, theme, cx, 23., 22., 18.))
        .into_any_element()
}

#[allow(clippy::too_many_arguments)]
fn card_top(
    c: &model::CardView,
    k: K,
    p: &Palette,
    theme: &Theme,
    cx: &App,
    who: f32,
    dot: f32,
    pill: f32,
) -> gpui::Div {
    let (pill_bg, pill_ink) = match c.tone {
        Tone::Warn => (p.amber_pill, theme.c("warn-ink")),
        Tone::Alert => (theme.c("red-bg"), theme.c("red-ink")),
        Tone::Quiet => (p.lilac_pill, theme.c("accent-text")),
    };
    div()
        .flex()
        .items_center()
        .gap(k.px(10.))
        .min_w_0()
        .whitespace_nowrap()
        .child(
            div()
                .min_w_0()
                .overflow_hidden()
                .text_ellipsis()
                .text_size(k.px(who))
                .font_weight(FontWeight::BOLD)
                .child(c.who.clone()),
        )
        .child(
            div()
                .flex_none()
                .text_size(k.px(dot))
                .font_weight(FontWeight::BOLD)
                .child("·"),
        )
        .child(
            div()
                .flex_none()
                .px(k.px(12.))
                .pt(k.px(3.))
                .pb(k.px(4.))
                .rounded(k.px(10.))
                .text_size(k.px(pill))
                .line_height(k.px(pill * 1.3))
                .bg(pill_bg)
                .text_color(pill_ink)
                .child(c.pill.clone()),
        )
        .when(c.hidden, |d| {
            d.child(
                div()
                    .flex_none()
                    .px(k.px(8.))
                    .pt(k.px(2.))
                    .pb(k.px(3.))
                    .rounded(k.px(8.))
                    .text_size(k.px(16.))
                    .bg(theme.c("chip"))
                    .text_color(theme.c("chip-ink"))
                    .child(t(cx, "strip.hidden")),
            )
        })
}

#[allow(clippy::too_many_arguments)]
fn shelf_bot(
    agent: &Agent,
    view: &WorkspaceView,
    now: u64,
    asleep: bool,
    slot: f32,
    bot: f32,
    name_px: f32,
    feet: f32,
    plate_h: f32,
    k: K,
    p: &Palette,
    theme: &Theme,
    cx: &App,
) -> AnyElement {
    let key = agent.key.clone();
    let ws = agent.workspace.clone();
    // TODO(chat lane): the robot itself (`src/robot/`), drawn big and with
    // its face; the avatar stands in until it lands.
    let robot = crate::ui::avatar::agent_avatar(agent, bot * 0.8 * k.0, false, cx);
    let mut slot_el = div()
        .id(SharedString::from(format!("bot-{}", agent.key)))
        .relative()
        .flex_none()
        .w(k.px(slot))
        .h_full()
        .cursor_pointer()
        .on_click(move |_, _, cx| go(Route::direct(Some(key.clone())), ws.clone(), cx))
        .child(
            div()
                .absolute()
                .left(k.px((slot - bot * 0.8) / 2.))
                .bottom(k.px(feet))
                .child(robot),
        );
    if !asleep {
        let a = &agent.activity;
        let micro = status::micro_state(a, now, &view.thresholds, cx);
        let words = status::status_text(a, agent.reports.len(), &micro, cx);
        let time = (!status::timeless(&a.state)).then(|| status::elapsed(now.saturating_sub(a.since_ms)));
        let text_color = if micro.long { theme.c("amber-ink") } else { p.ink };
        let line = match time {
            Some(t) => format!("{words}\u{a0}· {}", t.replace(' ', "\u{a0}")),
            None => words,
        };
        slot_el = slot_el.child(
            div()
                .absolute()
                .left_0()
                .right_0()
                .bottom_0()
                .h(k.px(plate_h))
                .flex()
                .flex_col()
                .items_center()
                .gap(k.px(7.))
                .pt(k.px(11.))
                .child(
                    div()
                        .max_w(k.px(slot - 10.))
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_ellipsis()
                        .text_size(k.px(name_px))
                        .line_height(k.px(name_px * 1.2))
                        .font_weight(FontWeight::BLACK)
                        .text_color(p.ink)
                        .child(agent.name.clone()),
                )
                .child(
                    div()
                        .flex()
                        .items_start()
                        .gap(k.px(7.))
                        .max_w(k.px(slot - 2.))
                        .text_size(k.px(21.))
                        .line_height(k.px(27.))
                        .text_color(text_color)
                        .child(
                            div()
                                .flex_none()
                                .size(k.px(30.))
                                .mt(k.px(-2.))
                                .rounded_full()
                                .bg(tok(theme, status::chip_token(&a.state), "chip"))
                                .flex()
                                .items_center()
                                .justify_center()
                                .child(ui::icon(status::icon(&a.state), 19. * k.0, theme.c("ink-2"))),
                        )
                        .child(div().min_w_0().line_clamp(2).child(line)),
                ),
        );
    }
    slot_el.into_any_element()
}
