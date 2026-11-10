//! Preferences › Appearance: the mode, the theme grid, the season and a
//! custom theme from two colours. Hovering or focusing a card previews it
//! over the whole app; clicking, Enter or Space chooses. Closing without
//! choosing restores what was chosen.

use super::widgets::{self, Kind, Mark, Size};
use crate::api::{self, ApiError};
use crate::derive::themes::{
    describe_refusal, picker_themes, season_day, season_name, slugify, swatch, theme_label, theme_notice,
    valid_theme_name,
};
use crate::i18n::{I18n, t, tv};
use crate::model::theme::{SeasonState, ThemeEntry, ThemePreview, ThemeRefusal, ThemeState};
use crate::theme::Theme;
use crate::ui;
use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, App, AppContext, ClipboardItem, Context, Entity, FocusHandle, FontWeight, Global, Hsla,
    InteractiveElement, IntoElement, KeyDownEvent, ParentElement, Render, Rgba, SharedString,
    StatefulInteractiveElement, Styled, Subscription, Task, Window, div, px, relative,
};
use gpui_kit::component::color_picker::{ColorPicker, ColorPickerEvent, ColorPickerState};
use gpui_kit::component::input::{InputEvent, InputState};
use serde_json::{Value, json};
use std::time::Duration;

/// Whether the painted theme is a preview, not the stored choice.
#[derive(Default)]
struct Previewing(bool);

impl Global for Previewing {}

fn previewing(cx: &App) -> bool {
    cx.try_global::<Previewing>().is_some_and(|p| p.0)
}

/// Paint `entry` over the stored state until [`end_preview`].
fn preview(real: &Value, entry: &Value, cx: &mut App) {
    let Some(id) = entry["id"].as_str() else { return };
    let mut state = real.clone();
    if let Some(all) = state["themes"].as_array_mut() {
        match all.iter_mut().find(|t| t["id"].as_str() == Some(id)) {
            Some(slot) => *slot = entry.clone(),
            None => all.push(entry.clone()),
        }
    }
    state["theme"] = json!(id);
    if state["season"].is_object() {
        state["season"]["active"] = json!(false);
    }
    let theme = Theme::from_state(state, cx.window_appearance());
    cx.set_global(theme);
    cx.set_global(Previewing(true));
    cx.refresh_windows();
}

/// Back to the stored theme, if a preview is painted.
pub fn end_preview(cx: &mut App) {
    if previewing(cx) {
        cx.set_global(Previewing(false));
        crate::theme::reload(cx);
    }
}

/// `POST /workspace/theme-*` (no workspace in the body), then the app
/// repaints from the stored state.
pub fn theme_write(action: &'static str, body: Value, cx: &mut App) -> Task<Result<Value, ApiError>> {
    cx.spawn(async move |cx| {
        let out = cx
            .background_spawn(async move { api::post(&format!("/workspace/{action}"), &body) })
            .await;
        cx.update(|cx| {
            cx.set_global(Previewing(false));
            crate::theme::reload(cx);
        });
        out
    })
}

/// A refused theme write: serve's typed reason, or its message.
fn refusal_of(e: &ApiError) -> Option<ThemeRefusal> {
    serde_json::from_value(e.body.get("refusal")?.clone()).ok()
}

/// `#rrggbb`, lower case, as `<input type=color>` gives it.
fn hex(c: Hsla) -> String {
    let rgb: Rgba = c.into();
    let ch = |v: f32| (v.clamp(0., 1.) * 255.).round() as u8;
    format!("#{:02x}{:02x}{:02x}", ch(rgb.r), ch(rgb.g), ch(rgb.b))
}

const MODES: [(&str, &str, &str); 3] = [
    ("light", "preferences.modeLight", "sun"),
    ("dark", "preferences.modeDark", "moon"),
    ("system", "preferences.modeSystem", "monitor"),
];

pub struct AppearancePage {
    /// serve's stored answer: what the picker shows as chosen.
    real: Value,
    state: Option<ThemeState>,
    /// The card keys move through.
    active: usize,
    status: Option<SharedString>,
    refusal: Option<ThemeRefusal>,
    season_error: Option<SharedString>,
    frame: String,
    accent: String,
    /// The unsaved custom theme serve derived from the two colours.
    draft: Option<Value>,
    draft_seq: u64,
    name: Entity<InputState>,
    code: Entity<InputState>,
    frame_picker: Entity<ColorPickerState>,
    accent_picker: Entity<ColorPickerState>,
    grid: FocusHandle,
    _subs: Vec<Subscription>,
}

impl AppearancePage {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let name = cx.new(|cx| InputState::new(window, cx).placeholder(t(cx, "preferences.themeNamePlaceholder")));
        let code = cx.new(|cx| InputState::new(window, cx).placeholder(t(cx, "preferences.themeCodePlaceholder")));
        let frame_picker = cx.new(|cx| ColorPickerState::new(window, cx));
        let accent_picker = cx.new(|cx| ColorPickerState::new(window, cx));
        let mut subs = vec![
            cx.subscribe_in(&code, window, |_, _, ev: &InputEvent, _, cx| {
                if matches!(ev, InputEvent::Change) {
                    cx.notify();
                }
            }),
            cx.subscribe_in(&frame_picker, window, |this, _, ev: &ColorPickerEvent, _, cx| {
                let ColorPickerEvent::Change(Some(c)) = ev else { return };
                let accent = this.accent.clone();
                this.edit_draft(hex(*c), accent, cx);
            }),
            cx.subscribe_in(&accent_picker, window, |this, _, ev: &ColorPickerEvent, _, cx| {
                let ColorPickerEvent::Change(Some(c)) = ev else { return };
                let frame = this.frame.clone();
                this.edit_draft(frame, hex(*c), cx);
            }),
        ];
        subs.push(cx.observe_global::<Theme>(|this, cx| {
            // A write or the system's appearance moved the stored state.
            if !previewing(cx) {
                this.reread(cx);
            }
            cx.notify();
        }));
        let mut this = AppearancePage {
            real: Value::Null,
            state: None,
            active: 0,
            status: None,
            refusal: None,
            season_error: None,
            frame: String::new(),
            accent: String::new(),
            draft: None,
            draft_seq: 0,
            name,
            code,
            frame_picker,
            accent_picker,
            grid: cx.focus_handle(),
            _subs: subs,
        };
        this.reread(cx);
        this.seed(window, cx);
        this
    }

    fn reread(&mut self, cx: &mut Context<Self>) {
        let real = cx.global::<Theme>().state.clone();
        if real == self.real {
            return;
        }
        self.state = serde_json::from_value(real.clone()).ok();
        self.real = real;
        let chosen = self.chosen_id();
        let themes = self.picker();
        if let Some(i) = themes.iter().position(|t| t.id == chosen) {
            self.active = i;
        }
    }

    /// The custom colours start from the chosen theme's light tokens.
    fn seed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(entry) = self.chosen_entry() else { return };
        let frame = entry.light.get("frame").cloned().unwrap_or_default();
        let accent = entry.light.get("accent").cloned().unwrap_or_default();
        if let Some(c) = crate::theme::parse_color(&frame) {
            self.frame_picker.update(cx, |p, cx| p.set_value(c, window, cx));
        }
        if let Some(c) = crate::theme::parse_color(&accent) {
            self.accent_picker.update(cx, |p, cx| p.set_value(c, window, cx));
        }
        self.frame = frame;
        self.accent = accent;
    }

    fn chosen_id(&self) -> String {
        self.state
            .as_ref()
            .map(|s| s.theme.clone())
            .unwrap_or_else(|| "ink".into())
    }

    fn chosen_entry(&self) -> Option<ThemeEntry> {
        let s = self.state.as_ref()?;
        s.themes.iter().find(|t| t.id == s.theme).cloned()
    }

    fn picker(&self) -> Vec<ThemeEntry> {
        self.state
            .as_ref()
            .map(|s| picker_themes(&s.themes).into_iter().cloned().collect())
            .unwrap_or_default()
    }

    fn raw_entry(&self, id: &str) -> Option<Value> {
        self.real["themes"]
            .as_array()?
            .iter()
            .find(|t| t["id"].as_str() == Some(id))
            .cloned()
    }

    fn fail(&mut self, e: ApiError) {
        self.refusal = refusal_of(&e);
        self.status = if self.refusal.is_some() {
            None
        } else {
            Some(e.message.into())
        };
    }

    fn after(
        &mut self,
        task: Task<Result<Value, ApiError>>,
        ok: impl FnOnce(&mut Self, Value, &mut Context<Self>) + 'static,
        cx: &mut Context<Self>,
    ) {
        cx.spawn(async move |this, cx| {
            let out = task.await;
            let _ = this.update(cx, |this, cx| {
                match out {
                    Ok(v) => ok(this, v, cx),
                    Err(e) => this.fail(e),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn choose(&mut self, entry: ThemeEntry, cx: &mut Context<Self>) {
        self.refusal = None;
        let label = theme_label(&entry, cx);
        let task = theme_write("theme-set", json!({ "theme": entry.id }), cx);
        self.after(
            task,
            move |this, _, cx| this.status = Some(tv(cx, "preferences.themeChosen", &[("name", &label)])),
            cx,
        );
    }

    fn set_mode(&mut self, mode: &'static str, label: SharedString, cx: &mut Context<Self>) {
        self.refusal = None;
        let task = theme_write("theme-mode", json!({ "mode": mode }), cx);
        self.after(
            task,
            move |this, _, cx| this.status = Some(tv(cx, "preferences.modeChosen", &[("label", &label)])),
            cx,
        );
    }

    fn hover_card(&mut self, i: usize, cx: &mut Context<Self>) {
        self.active = i;
        let themes = self.picker();
        if let Some(raw) = themes.get(i).and_then(|e| self.raw_entry(&e.id)) {
            preview(&self.real, &raw, cx);
        }
    }

    fn leave_grid(&mut self, cx: &mut Context<Self>) {
        if self.draft.is_none() {
            end_preview(cx);
        }
    }

    fn on_grid_key(&mut self, ev: &KeyDownEvent, _window: &mut Window, cx: &mut Context<Self>) {
        let n = self.picker().len();
        if n == 0 {
            return;
        }
        let i = self.active as i64;
        let next = match ev.keystroke.key.as_str() {
            "right" => i + 1,
            "left" => i - 1,
            "down" => i + 4,
            "up" => i - 4,
            "home" => 0,
            "end" => n as i64 - 1,
            "enter" | "space" => {
                if let Some(e) = self.picker().get(self.active).cloned() {
                    self.choose(e, cx);
                }
                cx.stop_propagation();
                return;
            }
            _ => return,
        };
        cx.stop_propagation();
        self.hover_card(next.clamp(0, n as i64 - 1) as usize, cx);
        cx.notify();
    }

    /// serve derives and checks the family as the colours move (120 ms debounce).
    fn edit_draft(&mut self, frame: String, accent: String, cx: &mut Context<Self>) {
        if frame == self.frame && accent == self.accent && self.draft.is_some() {
            return;
        }
        self.frame = frame.clone();
        self.accent = accent.clone();
        self.draft_seq += 1;
        let seq = self.draft_seq;
        cx.notify();
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Duration::from_millis(120)).await;
            if this.read_with(cx, |p, _| p.draft_seq != seq).unwrap_or(true) {
                return;
            }
            let body = json!({ "frame": frame, "accent": accent });
            let out = cx
                .background_spawn(async move { api::post("/workspace/theme-preview", &body) })
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.draft_seq != seq {
                    return;
                }
                match out {
                    Ok(v) => {
                        let parsed: ThemePreview = serde_json::from_value(v.clone()).unwrap_or_default();
                        if parsed.ok && v["theme"].is_object() {
                            this.refusal = None;
                            this.draft = Some(v["theme"].clone());
                            preview(&this.real, &v["theme"], cx);
                        } else {
                            this.draft = None;
                            this.refusal = parsed.refusal;
                            end_preview(cx);
                        }
                    }
                    Err(e) => this.fail(e),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn save_as(&mut self, cx: &mut Context<Self>) {
        let slug = slugify(&self.name.read(cx).value());
        if !valid_theme_name(&slug) {
            self.status = Some(t(cx, "preferences.enterThemeName"));
            cx.notify();
            return;
        }
        let file = format!("{slug}.toml");
        let task = theme_write(
            "theme-save",
            json!({ "name": slug, "frame": self.frame, "accent": self.accent }),
            cx,
        );
        self.after(
            task,
            move |this, _, cx| {
                this.draft = None;
                this.refusal = None;
                this.status = Some(tv(cx, "preferences.savedAs", &[("file", &file)]));
            },
            cx,
        );
    }

    fn shown(&self) -> Option<ThemeEntry> {
        match &self.draft {
            Some(d) => serde_json::from_value(d.clone()).ok(),
            None => self.chosen_entry(),
        }
    }

    fn copy(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(entry) = self.shown() else { return };
        cx.write_to_clipboard(ClipboardItem::new_string(entry.share.clone()));
        let name = theme_label(&entry, cx);
        self.status = Some(tv(cx, "preferences.codeCopied", &[("name", &name)]));
        let _ = window;
        cx.notify();
    }

    fn import(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let code = self.code.read(cx).value().trim().to_string();
        if code.is_empty() {
            return;
        }
        let input = self.code.clone();
        let task = theme_write("theme-import", json!({ "code": code }), cx);
        let window_handle = window.window_handle();
        cx.spawn(async move |this, cx| {
            let out = task.await;
            let _ = this.update(cx, |this, cx| {
                match out {
                    Ok(v) => {
                        this.refusal = None;
                        let name = v["theme"].as_str().unwrap_or_default().to_string();
                        this.status = Some(tv(cx, "preferences.imported", &[("name", &name)]));
                        let _ = window_handle.update(cx, |_, window, cx| {
                            input.update(cx, |s, cx| s.set_value("", window, cx));
                        });
                    }
                    Err(e) => this.fail(e),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn open_folder(&mut self, cx: &mut Context<Self>) {
        let task = theme_write("theme-open-folder", json!({}), cx);
        self.after(
            task,
            |this, _, cx| this.status = Some(t(cx, "preferences.themesFolderOpened")),
            cx,
        );
    }

    fn set_season(&mut self, choice: &'static str, cx: &mut Context<Self>) {
        self.season_error = None;
        let task = theme_write("theme-season", json!({ "season": choice }), cx);
        cx.spawn(async move |this, cx| {
            let failed = task.await.is_err();
            let _ = this.update(cx, |this, cx| {
                if failed {
                    this.season_error = Some(t(cx, "preferences.couldNotSave"));
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn set_decorations(&mut self, on: bool, cx: &mut Context<Self>) {
        self.season_error = None;
        let task = theme_write("theme-decorations", json!({ "on": on }), cx);
        cx.spawn(async move |this, cx| {
            let failed = task.await.is_err();
            let _ = this.update(cx, |this, cx| {
                if failed {
                    this.season_error = Some(t(cx, "preferences.couldNotSave"));
                }
                cx.notify();
            });
        })
        .detach();
    }

    // -----------------------------------------------------------------------

    fn render_mode(&self, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let mode = self
            .state
            .as_ref()
            .map(|s| s.mode.clone())
            .unwrap_or_else(|| "light".into());
        let disabled = self.state.is_none();
        div()
            .child(widgets::legend("sun-moon", t(cx, "preferences.mode"), theme))
            .child(widgets::hint(t(cx, "preferences.modeHint"), theme))
            .child(widgets::mode_row(
                MODES
                    .iter()
                    .map(|(id, key, icon)| {
                        let label = t(cx, key);
                        let l2 = label.clone();
                        let id = *id;
                        widgets::mode_box(
                            SharedString::from(format!("mode-{id}")),
                            Mark::Radio,
                            mode == id,
                            Some(icon),
                            label,
                            disabled,
                            theme,
                            cx.listener(move |this, _, _, cx| this.set_mode(id, l2.clone(), cx)),
                        )
                        .into_any_element()
                    })
                    .collect(),
            ))
            .into_any_element()
    }

    #[allow(clippy::too_many_arguments)]
    fn render_card(
        &self,
        i: usize,
        entry: &ThemeEntry,
        chosen: bool,
        focused: bool,
        dark: bool,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let s = swatch(entry, dark);
        let accent = theme.c("accent");
        let hover = theme.c("line-hover");
        let bar =
            |c: Hsla, w: gpui::DefiniteLength, h: f32, o: f32| div().h(px(h)).w(w).rounded(px(3.)).bg(c).opacity(o);
        let entry2 = entry.clone();
        div()
            .id(SharedString::from(format!("theme-card-{}", entry.id)))
            .flex()
            .flex_col()
            .gap(px(8.))
            .p(px(8.))
            .rounded(px(10.))
            .bg(theme.c("surface-raised"))
            .border_1()
            .cursor_pointer()
            .border_color(if chosen { accent } else { theme.c("line-strong") })
            .when(!chosen, |d| d.hover(move |st| st.border_color(hover)))
            .when(chosen, |d| d.shadow(vec![widgets::ring_shadow(accent, 2., false)]))
            .when(focused && !chosen, |d| {
                d.shadow(vec![widgets::ring_shadow(theme.c("focus"), 2., false)])
            })
            .child(
                div()
                    .flex()
                    .h(px(64.))
                    .rounded(px(6.))
                    .overflow_hidden()
                    .border_1()
                    .border_color(theme.c("panel-ring"))
                    .bg(s.frame)
                    .child(
                        div()
                            .w(px(22.))
                            .flex()
                            .flex_col()
                            .items_center()
                            .gap(px(6.))
                            .pt(px(8.))
                            .child(div().size(px(10.)).rounded(px(3.)).bg(s.rail_ink).opacity(0.7))
                            .child(div().size(px(10.)).rounded(px(3.)).bg(s.rail_ink).opacity(0.7)),
                    )
                    .child(
                        div()
                            .flex_1()
                            .mt(px(6.))
                            .mr(px(6.))
                            .rounded_t(px(4.))
                            .flex()
                            .flex_col()
                            .gap(px(5.))
                            .p(px(8.))
                            .bg(s.sidebar)
                            .child(bar(s.ink, relative(0.7), 6., 0.55))
                            .child(bar(s.accent, relative(1.), 9., 1.))
                            .child(bar(s.ink, relative(0.7), 6., 0.55)),
                    ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .font_weight(FontWeight::BOLD)
                    .text_size(px(14.))
                    .child(theme_label(entry, cx))
                    .when(!entry.builtin, |d| {
                        d.child(
                            div()
                                .px(px(6.))
                                .rounded(px(4.))
                                .text_size(px(11.))
                                .font_weight(FontWeight::BOLD)
                                .bg(theme.c("chip"))
                                .text_color(theme.c("chip-ink"))
                                .child(if entry.stale {
                                    t(cx, "preferences.themeFileRejected")
                                } else {
                                    t(cx, "preferences.themeFile")
                                }),
                        )
                    }),
            )
            .on_hover(cx.listener(move |this, on: &bool, _, cx| {
                if *on {
                    this.hover_card(i, cx);
                }
            }))
            .on_click(cx.listener(move |this, _, window, cx| {
                this.grid.focus(window, cx);
                this.active = i;
                this.choose(entry2.clone(), cx);
            }))
            .into_any_element()
    }

    fn render_themes(&self, theme: &Theme, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let themes = self.picker();
        let chosen = self.chosen_id();
        let dark = theme.dark;
        let focused = self.grid.is_focused(window);
        let cards: Vec<AnyElement> = themes
            .iter()
            .enumerate()
            .map(|(i, e)| self.render_card(i, e, e.id == chosen, focused && i == self.active, dark, theme, cx))
            .collect();
        let mut rows = Vec::new();
        let mut it = cards.into_iter().peekable();
        while it.peek().is_some() {
            let mut row = div().flex().gap(px(14.));
            for _ in 0..4 {
                row = row.child(div().flex_1().min_w_0().children(it.next()));
            }
            rows.push(row);
        }
        let errors = self.state.as_ref().map(|s| s.errors.clone()).unwrap_or_default();
        let notice = self.state.as_ref().and_then(|s| theme_notice(s, cx));
        div()
            .child(widgets::legend("swatch-book", t(cx, "preferences.themes"), theme))
            .child(widgets::hint(t(cx, "preferences.themesHint"), theme))
            .child(
                div()
                    .id("theme-grid")
                    .track_focus(&self.grid)
                    .flex()
                    .flex_col()
                    .gap(px(14.))
                    .children(rows)
                    .on_key_down(cx.listener(Self::on_grid_key))
                    .on_hover(cx.listener(|this, on: &bool, _, cx| {
                        if !*on {
                            this.leave_grid(cx);
                        }
                    })),
            )
            .when(!errors.is_empty(), |d| {
                d.child(
                    div()
                        .mt(px(14.))
                        .px(px(12.))
                        .py(px(10.))
                        .rounded(px(8.))
                        .bg(theme.c("red-bg"))
                        .text_color(theme.c("banner-ink"))
                        .text_size(px(13.))
                        .child(
                            div()
                                .font_weight(FontWeight::BOLD)
                                .child(t(cx, "preferences.rejectedFiles")),
                        )
                        .children(errors.iter().map(|e| {
                            div()
                                .mt(px(6.))
                                .flex()
                                .gap(px(8.))
                                .child("•")
                                .child(div().font_family(ui::MONO).child(describe_refusal(e, cx)))
                        })),
                )
            })
            .when_some(notice, |d, n| d.child(div().child(widgets::hint(n, theme))))
            .into_any_element()
    }

    fn render_season(&self, season: &SeasonState, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let name = season_name(&season.id, cx);
        let locale = cx.global::<I18n>().locale;
        let choices = [
            ("auto", "preferences.seasonAuto"),
            ("on", "preferences.seasonOn"),
            ("off", "preferences.seasonOff"),
        ];
        let decorations = season.decorations;
        div()
            .child(widgets::legend(
                "snowflake",
                tv(cx, "preferences.seasonTitle", &[("name", &name)]),
                theme,
            ))
            .child(
                div().mt(px(12.)).child(widgets::mode_row(
                    choices
                        .iter()
                        .map(|(id, key)| {
                            let id = *id;
                            widgets::mode_box(
                                SharedString::from(format!("season-{id}")),
                                Mark::Radio,
                                season.choice == id,
                                None,
                                t(cx, key),
                                false,
                                theme,
                                cx.listener(move |this, _, _, cx| this.set_season(id, cx)),
                            )
                            .into_any_element()
                        })
                        .collect(),
                )),
            )
            .child(div().child(widgets::hint(
                tv(
                    cx,
                    "preferences.seasonHint",
                    &[
                        ("name", &name),
                        ("from", &season_day(&season.from, locale)),
                        ("to", &season_day(&season.to, locale)),
                    ],
                ),
                theme,
            )))
            .child(
                div().child(
                    widgets::mode_box(
                        "season-decorations",
                        Mark::Check,
                        decorations,
                        None,
                        t(cx, "preferences.seasonDecorations"),
                        false,
                        theme,
                        cx.listener(move |this, _, _, cx| this.set_decorations(!decorations, cx)),
                    )
                    .w_full(),
                ),
            )
            .child(widgets::hint(t(cx, "preferences.seasonDecorationsHint"), theme))
            .when_some(self.season_error.clone(), |d, e| d.child(widgets::error(e, theme)))
            .into_any_element()
    }

    fn render_custom(&self, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let disabled = self.state.is_none();
        let row = || div().flex().flex_wrap().items_center().gap(px(12.)).mt(px(12.));
        let colour = |picker: &Entity<ColorPickerState>, label: SharedString, value: &str| {
            div()
                .flex()
                .items_center()
                .gap(px(10.))
                .font_weight(FontWeight::BOLD)
                .child(ColorPicker::new(picker))
                .child(
                    div().flex().gap(px(4.)).child(label).child(
                        div()
                            .font_family(ui::MONO)
                            .text_size(px(13.))
                            .font_weight(FontWeight::NORMAL)
                            .child(value.to_string()),
                    ),
                )
        };
        let has_code = !self.code.read(cx).value().trim().is_empty();
        let status: Option<AnyElement> = match (&self.refusal, &self.status) {
            (Some(r), _) => Some(widgets::error(describe_refusal(r, cx), theme).into_any_element()),
            (None, Some(s)) => Some(s.clone().into_any_element()),
            _ => None,
        };
        div()
            .child(widgets::legend("pipette", t(cx, "preferences.customTheme"), theme))
            .child(widgets::hint(t(cx, "preferences.customThemeHint"), theme))
            .when(!self.frame.is_empty() && !self.accent.is_empty(), |d| {
                d.child(
                    row()
                        .child(colour(&self.frame_picker, t(cx, "preferences.frame"), &self.frame))
                        .child(colour(&self.accent_picker, t(cx, "preferences.accent"), &self.accent)),
                )
            })
            .child(
                row()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(8.))
                            .font_weight(FontWeight::BOLD)
                            .child(t(cx, "preferences.name"))
                            .child(div().w(px(220.)).child(widgets::text_input(&self.name, theme))),
                    )
                    .child(widgets::button(
                        "theme-save",
                        t(cx, "preferences.saveAs"),
                        Kind::Primary,
                        Size::Normal,
                        None,
                        disabled,
                        theme,
                        cx.listener(|this, _, _, cx| this.save_as(cx)),
                    ))
                    .child(widgets::button(
                        "theme-copy",
                        t(cx, "preferences.copyShareCode"),
                        Kind::Plain,
                        Size::Normal,
                        None,
                        self.shown().is_none(),
                        theme,
                        cx.listener(|this, _, window, cx| this.copy(window, cx)),
                    )),
            )
            .child(
                row()
                    .child(
                        div()
                            .flex()
                            .flex_1()
                            .items_center()
                            .gap(px(8.))
                            .font_weight(FontWeight::BOLD)
                            .child(div().flex_none().child(t(cx, "preferences.pasteCode")))
                            .child(
                                div()
                                    .flex_1()
                                    .font_family(ui::MONO)
                                    .child(widgets::text_input(&self.code, theme)),
                            ),
                    )
                    .child(widgets::button(
                        "theme-import",
                        t(cx, "preferences.importTheme"),
                        Kind::Plain,
                        Size::Normal,
                        None,
                        disabled || !has_code,
                        theme,
                        cx.listener(|this, _, window, cx| this.import(window, cx)),
                    )),
            )
            .child(
                row()
                    .child(widgets::button(
                        "theme-folder",
                        t(cx, "preferences.openThemesFolder"),
                        Kind::Plain,
                        Size::Normal,
                        Some("folder-open"),
                        disabled,
                        theme,
                        cx.listener(|this, _, _, cx| this.open_folder(cx)),
                    ))
                    .when_some(self.state.as_ref().map(|s| s.dir.clone()), |d, dir| {
                        d.child(
                            div()
                                .font_family(ui::MONO)
                                .text_size(px(13.))
                                .text_color(theme.c("ink-3"))
                                .child(dir),
                        )
                    }),
            )
            .child(widgets::status_line(status, theme))
            .into_any_element()
    }
}

impl Render for AppearancePage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.global::<Theme>().clone();
        let mut blocks = vec![self.render_mode(&theme, cx), self.render_themes(&theme, window, cx)];
        if let Some(season) = self.state.as_ref().and_then(|s| s.season.clone()) {
            blocks.push(self.render_season(&season, &theme, cx));
        }
        blocks.push(self.render_custom(&theme, cx));
        div()
            .when(self.state.is_none(), |d| {
                d.child(widgets::banner(t(cx, "preferences.themesUnavailable"), &theme))
            })
            .child(widgets::sections(blocks, &theme))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_rounds_like_the_colour_input() {
        let c: Hsla = Rgba {
            r: 194. / 255.,
            g: 79. / 255.,
            b: 56. / 255.,
            a: 1.,
        }
        .into();
        assert_eq!(hex(c), "#c24f38");
    }
}
