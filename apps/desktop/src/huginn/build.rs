//! The walk that turns a view into elements: visibility, repeats, resolved
//! props, and the display components (layout, text, data, work). Charts are
//! in `charts.rs`; the components that ask are in `ask.rs`.

use super::expr::{self, Ctx};
use super::scale::{self, format_plain};
use super::spec::{Element, Spec};
use super::{Given, HuginnView};
use crate::i18n::{Locale, t, tv};
use crate::model::Attachment;
use crate::theme::{Theme, parse_color};
use crate::ui::{self, control, icon_size, leading, radius, size, space, weight};
use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, App, BoxShadow, Context, Div, ElementId, EntityId, Font, FontWeight, Hsla,
    InteractiveElement, IntoElement, ParentElement, Pixels, SharedString, Stateful,
    StatefulInteractiveElement, Styled, Subscription, TextRun, Window, canvas, div, font, hsla,
    point, px, relative,
};
use serde_json::{Map, Value};
use std::cell::Cell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::Arc;

/// The tokens a view paints with, looked up once per render.
pub(super) struct Pal {
    colors: HashMap<&'static str, Hsla>,
    pub dark: bool,
}

const TOKENS: &[&str] = &[
    "surface",
    "surface-2",
    "surface-raised",
    "ink",
    "ink-2",
    "ink-3",
    "ink-4",
    "line",
    "line-strong",
    "line-hover",
    "chip",
    "chip-ink",
    "accent",
    "accent-text",
    "accent-soft",
    "accent-ink",
    "accent-ink-2",
    "accent-hover",
    "accent-deep",
    "green-edge",
    "green-ink",
    "green-bg",
    "amber-edge",
    "amber-ink",
    "amber-bg",
    "red-edge",
    "red-ink",
    "red-bg",
    "link",
    "focus",
    "btn-hover",
    "panel-ring",
    "control",
    "tint",
    "hover",
    "selected",
    "menu-hover",
    "menu-hover-ink",
];

/// `--hb-series-1..8`: one set for every theme.
pub(super) const SERIES: [u32; 8] = [
    0x2a78d6, 0xeb6834, 0x1baf7a, 0xeda100, 0xe87ba4, 0x008300, 0x6250d6, 0xe34948,
];

pub(super) fn series(i: usize) -> Hsla {
    gpui::rgb(SERIES[i % SERIES.len()]).into()
}

impl Pal {
    pub fn new(theme: &Theme) -> Pal {
        let colors = TOKENS
            .iter()
            .map(|name| {
                (
                    *name,
                    theme
                        .raw(name)
                        .and_then(parse_color)
                        .unwrap_or(hsla(0.83, 1.0, 0.5, 1.0)),
                )
            })
            .collect();
        Pal {
            colors,
            dark: theme.dark,
        }
    }

    pub fn c(&self, name: &str) -> Hsla {
        self.colors
            .get(name)
            .copied()
            .unwrap_or(hsla(0.83, 1.0, 0.5, 1.0))
    }

    /// (edge, ink, background) of a `data-tone`.
    pub fn tone(&self, tone: &str) -> (Hsla, Hsla, Hsla) {
        match tone {
            "success" => (
                self.c("green-edge"),
                self.c("green-ink"),
                self.c("green-bg"),
            ),
            "warning" => (
                self.c("amber-edge"),
                self.c("amber-ink"),
                self.c("amber-bg"),
            ),
            "danger" => (self.c("red-edge"), self.c("red-ink"), self.c("red-bg")),
            // Emphasis without a state: a neutral wash, a shade from plain (C2).
            "accent" | "info" => (self.c("ink-3"), self.c("ink"), self.c("tint")),
            _ => (self.c("line-strong"), self.c("chip-ink"), self.c("chip")),
        }
    }
}

/// `.hb-huginn-card { border-left: 3px solid var(--huginn-tone) }`.
const CARD_BAND: f32 = 3.;

/// The colour of a card's leading band (`--huginn-tone`): a state's edge,
/// the accent for `info`, `line-strong` when it has no tone.
fn card_band(tone: &str, pal: &Pal) -> Hsla {
    match card_band_token(tone) {
        Some(token) => pal.c(token),
        None => pal.tone(tone).0,
    }
}

/// The token behind a band that does not follow [`Pal::tone`]'s edge.
fn card_band_token(tone: &str) -> Option<&'static str> {
    match tone {
        "info" | "accent" => Some("accent"),
        "success" | "warning" | "danger" => None,
        _ => Some("line-strong"),
    }
}

/// A form field's value.
#[derive(Debug, Clone, PartialEq)]
pub(super) enum FieldValue {
    Text(String),
    List(Vec<String>),
    Bool(bool),
}

impl FieldValue {
    pub fn text(&self) -> &str {
        match self {
            FieldValue::Text(s) => s,
            _ => "",
        }
    }

    pub fn filled(&self) -> bool {
        match self {
            FieldValue::Text(s) => !s.is_empty(),
            FieldValue::List(l) => !l.is_empty(),
            FieldValue::Bool(_) => true,
        }
    }
}

/// One registered field of a Form.
#[derive(Debug, Clone)]
pub(super) struct FieldReg {
    pub kind: String,
    pub value: FieldValue,
    pub required: bool,
    pub options: Vec<String>,
    pub default_text: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub(super) enum Sent {
    #[default]
    Idle,
    Sending,
    Failed(String),
}

#[derive(Default)]
pub(super) struct AskUi {
    pub picked: Vec<usize>,
    pub sent: Sent,
    pub local: Option<Given>,
    /// Form fields by id, and their document order in the last render.
    pub fields: HashMap<String, FieldReg>,
    pub order: Vec<String>,
}

#[derive(Default, Clone)]
pub(super) struct TableUi {
    pub sort: Option<(usize, bool)>,
    pub all: bool,
    /// (content fingerprint, max-content and min-content widths per column)
    pub measured: Option<(u64, Vec<f32>, Vec<f32>)>,
}

/// Everything a view remembers between renders that is not its state.
#[derive(Default)]
pub(super) struct Ui {
    pub tabs: HashMap<String, usize>,
    pub open: HashMap<String, (bool, bool)>,
    pub tables: HashMap<String, TableUi>,
    pub full: HashSet<String>,
    pub hover: HashMap<String, usize>,
    pub widths: HashMap<String, Rc<Cell<f32>>>,
    pub bounds: HashMap<String, Rc<Cell<gpui::Bounds<Pixels>>>>,
    pub compare: HashMap<String, (bool, f32)>,
    pub asks: HashMap<String, AskUi>,
    pub inputs: HashMap<String, gpui::Entity<gpui_kit::component::input::InputState>>,
    pub areas: HashMap<String, gpui::Entity<gpui_kit::component::input::TextareaState>>,
    pub select_open: Option<String>,
    pub drag: Option<String>,
    pub images: HashMap<String, super::ImageLoad>,
}

/// Where an element renders: inside a repeat item, or not.
#[derive(Clone, Copy)]
pub(super) struct Scope<'a> {
    pub item: Option<&'a Value>,
    pub index: Option<usize>,
}

/// The Form being rendered, for the fields inside it.
#[derive(Clone)]
pub(super) struct FormFrame {
    pub id: String,
    pub disabled: bool,
}

/// Who an answer goes to; `None` when this message cannot be answered.
#[derive(Clone)]
pub(super) struct Target {
    pub agent: String,
    pub reply_to: String,
    pub author: String,
}

pub(super) struct B<'a, 'w> {
    pub spec: &'a Spec,
    pub state: &'a Value,
    pub ui: &'a mut Ui,
    pub pal: &'a Pal,
    pub locale: Locale,
    pub window: &'a mut Window,
    pub cx: &'a mut Context<'w, HuginnView>,
    pub entity: EntityId,
    pub file: &'a str,
    pub files: &'a [Attachment],
    pub target: Option<Target>,
    pub given: &'a HashMap<String, Given>,
    /// State writes fields make when they mount, applied after the walk.
    pub publish: Vec<(String, Value)>,
    /// `form/field` keys rendered this frame; the rest unregister.
    pub seen: HashSet<String>,
    pub form: Option<FormFrame>,
    pub subs: Vec<Subscription>,
}

pub(super) fn s<'v>(props: &'v Value, key: &str) -> Option<&'v str> {
    props.get(key).and_then(Value::as_str)
}

pub(super) fn n(props: &Value, key: &str) -> Option<f64> {
    props.get(key).and_then(Value::as_f64)
}

pub(super) fn flag(props: &Value, key: &str) -> bool {
    props.get(key).and_then(Value::as_bool).unwrap_or(false)
}

pub(super) fn arr<'v>(props: &'v Value, key: &str) -> &'v [Value] {
    props
        .get(key)
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[])
}

/// A scoped id: an element key, plus the repeat index when inside one.
fn sid(key: &str, scope: &Scope<'_>) -> String {
    match scope.index {
        Some(i) => format!("{key}@{i}"),
        None => key.to_string(),
    }
}

pub(super) fn eid(parts: impl Into<SharedString>) -> ElementId {
    ElementId::Name(parts.into())
}

/// Text in the view's face.
pub(super) fn txt(s: impl Into<SharedString>, size: f32, weight: FontWeight, color: Hsla) -> Div {
    div()
        .text_size(px(size))
        .line_height(px(leading::for_size(size)))
        .font_weight(weight)
        .text_color(color)
        .child(s.into())
}

/// An icon on the first line of the text beside it, whatever follows below.
pub(super) fn lead(icon: impl IntoElement, line: f32) -> Div {
    div()
        .flex()
        .flex_none()
        .items_center()
        .h(px(line))
        .child(icon)
}

/// The keyboard focus ring (F1) on a clickable part of a view.
pub(super) fn focusable(el: Stateful<Div>, pal: &Pal, inset: bool) -> Stateful<Div> {
    let ring = BoxShadow {
        color: pal.c("focus"),
        offset: point(px(0.), px(0.)),
        blur_radius: px(0.),
        spread_radius: px(space::HAIR),
        inset,
    };
    el.focusable()
        .tab_stop(true)
        .focus_visible(move |s| s.shadow(vec![ring]))
}

/// A state mark: a timeline's dot, a card's tone.
pub(super) const DOT: f32 = space::MD;
/// A code block, diff or log scrolls beyond this.
const CODE_MAX_H: f32 = 420.;
/// The label column of a key/value list is at least this wide.
const KEY_MIN_W: f32 = 96.;
/// An image is at most this tall; while it loads its box is this size.
const IMAGE_MAX_H: f32 = 480.;
const PLACEHOLDER_W: f32 = 320.;
const PLACEHOLDER_H: f32 = 120.;

/// Collapse whitespace as an HTML block with `white-space: normal` does.
pub(super) fn collapse(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

impl<'a, 'w> B<'a, 'w> {
    pub fn tr(&self, key: &str) -> SharedString {
        t(self.cx, &format!("huginn.{key}"))
    }

    pub fn trv(&self, key: &str, vars: &[(&str, &str)]) -> SharedString {
        tv(self.cx, &format!("huginn.{key}"), vars)
    }

    pub fn num(&self, value: Option<f64>, unit: Option<&str>) -> String {
        scale::format_number(value, unit, self.locale, &self.tr("noData"))
    }

    pub fn percent(&self, value: f64) -> SharedString {
        let v = format_plain(value, self.locale);
        self.trv("percent", &[("value", &v)])
    }

    /// The width a container had at its last paint, and the probe that
    /// measures it again (place it as a child of a `relative()` container).
    pub fn width_probe(&mut self, key: &str, initial: f32) -> (f32, AnyElement) {
        let cell = self
            .ui
            .widths
            .entry(key.to_string())
            .or_insert_with(|| Rc::new(Cell::new(initial)))
            .clone();
        let width = cell.get();
        let entity = self.entity;
        let probe = canvas(
            move |bounds, _window, cx: &mut App| {
                let w = f32::from(bounds.size.width);
                if (w - cell.get()).abs() > 0.5 {
                    cell.set(w);
                    cx.notify(entity);
                }
            },
            |_, _, _, _| {},
        )
        .absolute()
        .top_0()
        .left_0()
        .size_full()
        .into_any_element();
        (width, probe)
    }

    /// Where an element was painted, for pointer maths.
    pub fn bounds_cell(&mut self, key: &str) -> Rc<Cell<gpui::Bounds<Pixels>>> {
        self.ui.bounds.entry(key.to_string()).or_default().clone()
    }

    /// The width of one line of text.
    pub fn measure(
        &mut self,
        text: &str,
        size: f32,
        weight: FontWeight,
        family: &'static str,
    ) -> f32 {
        if text.is_empty() {
            return 0.0;
        }
        let run = TextRun {
            len: text.len(),
            font: Font {
                weight,
                ..font(family)
            },
            color: self.pal.c("ink"),
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        let line = self.window.text_system().shape_line(
            SharedString::from(text.to_string()),
            px(size),
            &[run],
            None,
        );
        f32::from(line.width)
    }

    /// One element and its subtree, or `None` when it is hidden.
    pub fn element(&mut self, key: &str, scope: Scope<'a>) -> Option<AnyElement> {
        let spec = self.spec;
        let el = spec.elements.get(key)?;
        let ctx = Ctx {
            state: self.state,
            item: scope.item,
            index: scope.index,
        };
        let (gate, filter) = if el.repeat.is_some() && scope.item.is_none() {
            expr::split_repeat(el.visible.as_ref())
        } else {
            (el.visible.clone(), None)
        };
        if !expr::visible(gate.as_ref(), &ctx) {
            return None;
        }
        let dynamic = el.props.values().any(|v| {
            v.as_object()
                .is_some_and(|o| o.keys().any(|k| k.starts_with('$')))
        });
        let props = if dynamic {
            expr::resolve_props(&el.props, &ctx)
        } else {
            Value::Object(el.props.clone())
        };
        if dynamic && harbard_cli::huginn::resolved_props_problem(&el.kind, &props).is_some() {
            return Some(self.error_line(self.tr("componentInvalid")));
        }
        let id = sid(key, &scope);
        let children = Children { el, scope, filter };
        Some(self.component(el, &props, &id, children))
    }

    /// The red line a broken component shows instead of itself.
    pub fn error_line(&self, text: impl Into<SharedString>) -> AnyElement {
        txt(text, size::SMALL, weight::REGULAR, self.pal.c("red-ink")).into_any_element()
    }

    /// The rendered children of an element (a repeat renders them once per item).
    pub fn children(&mut self, kids: &Children<'a>) -> Vec<AnyElement> {
        let mut out = Vec::new();
        match &kids.el.repeat {
            Some(repeat) if kids.scope.item.is_none() => {
                let items: &'a [Value] = match expr::get(self.state, &repeat.state_path) {
                    None => &[],
                    Some(Value::Array(items)) => items.as_slice(),
                    // json-render throws here, and its boundary shows nothing.
                    Some(_) => return out,
                };
                let limit = items.len().min(200);
                for (i, item) in items[..limit].iter().enumerate() {
                    let ctx = Ctx {
                        state: self.state,
                        item: Some(item),
                        index: Some(i),
                    };
                    if kids.filter.is_some() && !expr::visible(kids.filter.as_ref(), &ctx) {
                        continue;
                    }
                    for child in &kids.el.children {
                        if let Some(e) = self.element(
                            child,
                            Scope {
                                item: Some(item),
                                index: Some(i),
                            },
                        ) {
                            out.push(e);
                        }
                    }
                }
            }
            _ => {
                for child in &kids.el.children {
                    if let Some(e) = self.element(child, kids.scope) {
                        out.push(e);
                    }
                }
            }
        }
        out
    }

    fn component(
        &mut self,
        el: &'a Element,
        p: &Value,
        id: &str,
        kids: Children<'a>,
    ) -> AnyElement {
        match el.kind.as_str() {
            "Stack" => self.stack(p, &kids),
            "Grid" => self.grid(p, id, &kids),
            "Section" => self.section(p, &kids),
            "Card" => self.card(p, &kids),
            "Tabs" => self.tabs(p, id, &kids),
            "Disclosure" => self.disclosure(p, id, &kids),
            "Separator" => div()
                .border_t_1()
                .border_color(self.pal.c("line"))
                .into_any_element(),
            "Heading" => self.heading(p),
            "Text" => self.text(p),
            "Markdown" => self.markdown(p, id),
            "Callout" => self.callout(p),
            "Quote" => self.quote(p),
            "Badge" => self.badge(p),
            "KeyValue" => self.key_value(p),
            "List" => self.list(p),
            "Link" => self.link(p, id),
            "Code" => self.code(p, id),
            "Metric" => self.metric(p),
            "Table" => self.table(p, id),
            "Progress" => self.progress(p),
            "Timeline" => self.timeline(p),
            "Steps" => self.steps(p, false),
            "Checklist" => self.steps(p, true),
            "BarChart" => self.bar_chart(p, id),
            "LineChart" => self.line_chart(p, id),
            "Sparkline" => self.sparkline(p, id),
            "Donut" => self.donut(p, id),
            "Heatmap" => self.heatmap(p, id),
            "Diff" => self.diff(p, id),
            "FileTree" => self.file_tree(p),
            "Terminal" => self.terminal(p, id),
            "Image" => self.image(p),
            "BeforeAfter" => self.before_after(p, id),
            "Segmented" => self.segmented(p, id),
            "Switch" => self.switch(p, id),
            "Choice" => self.choice(p, id),
            "Form" => self.form_el(p, id, &kids),
            "TextField" | "NumberField" | "DateField" => self.text_field(&el.kind, p, id),
            "SelectField" => self.select_field(p, id),
            "RadioField" => self.radio_field(p, id),
            "CheckboxField" => self.checkbox_field(p, id),
            "MultiSelectField" => self.multi_field(p, id),
            "SliderField" => self.slider_field(p, id),
            _ => div().into_any_element(),
        }
    }

    // ---------------------------------------------------------------- layout

    fn gap(p: &Value) -> f32 {
        match s(p, "gap") {
            Some("sm") => space::SM,
            Some("lg") => space::GUTTER,
            _ => space::LG,
        }
    }

    fn stack(&mut self, p: &Value, kids: &Children<'a>) -> AnyElement {
        let horizontal = s(p, "direction") == Some("horizontal");
        let children = self.children(kids);
        div()
            .flex()
            .min_w_0()
            .when(horizontal, |d| d.flex_row())
            .when(!horizontal, |d| d.flex_col())
            .when(flag(p, "wrap"), |d| d.flex_wrap())
            .map(|d| match s(p, "align") {
                Some("start") => d.items_start(),
                Some("center") => d.items_center(),
                Some("end") => d.items_end(),
                _ => d,
            })
            .gap(px(Self::gap(p)))
            .children(children)
            .into_any_element()
    }

    /// `repeat(auto-fit, minmax(min(100%, 640px / columns - 12px), 1fr))`.
    fn grid(&mut self, p: &Value, id: &str, kids: &Children<'a>) -> AnyElement {
        let columns = n(p, "columns").unwrap_or(1.0).max(1.0);
        let gap = Self::gap(p);
        let (width, probe) = self.width_probe(&format!("{id}#grid"), 640.);
        let children = self.children(kids);
        let track = (640.0 / columns as f32 - space::LG).min(width).max(1.0);
        let fit = (((width + gap) / (track + gap)).floor() as usize).max(1);
        let per_row = fit.min(children.len().max(1));
        let mut rows: Vec<AnyElement> = Vec::new();
        let mut it = children.into_iter().peekable();
        while it.peek().is_some() {
            let mut cells: Vec<AnyElement> = Vec::new();
            for _ in 0..per_row {
                match it.next() {
                    Some(c) => cells.push(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .child(c)
                            .into_any_element(),
                    ),
                    None => cells.push(div().flex_1().min_w_0().into_any_element()),
                }
            }
            rows.push(
                div()
                    .flex()
                    .flex_row()
                    .gap(px(gap))
                    .children(cells)
                    .into_any_element(),
            );
        }
        div()
            .relative()
            .flex()
            .flex_col()
            .gap(px(gap))
            .min_w_0()
            .child(probe)
            .children(rows)
            .into_any_element()
    }

    fn section(&mut self, p: &Value, kids: &Children<'a>) -> AnyElement {
        let children = self.children(kids);
        div()
            .flex()
            .flex_col()
            .gap(px(space::MD))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .child(txt(
                        s(p, "title").unwrap_or_default().to_string(),
                        size::BODY,
                        weight::STRONG,
                        self.pal.c("ink"),
                    ))
                    .when_some(s(p, "description"), |d, desc| {
                        d.child(txt(
                            desc.to_string(),
                            size::SMALL,
                            weight::REGULAR,
                            self.pal.c("ink-3"),
                        ))
                    }),
            )
            .children(children)
            .into_any_element()
    }

    /// `.hb-huginn-card`: a card on `surface-2` with a hairline edge and a
    /// 3 px band at its leading edge in its tone (§3.1), the tone also as a
    /// dot beside the title.
    fn card(&mut self, p: &Value, kids: &Children<'a>) -> AnyElement {
        let tone = s(p, "tone").unwrap_or("neutral");
        let (edge, _, _) = self.pal.tone(tone);
        let band = card_band(tone, self.pal);
        let children = self.children(kids);
        let ink3 = self.pal.c("ink-3");
        div()
            .relative()
            .flex()
            .flex_col()
            .gap(px(space::MD))
            .p(px(space::LG))
            .pl(px(space::LG + CARD_BAND))
            .border_1()
            .border_color(self.pal.c("line"))
            .rounded(px(radius::MD))
            .bg(self.pal.c("surface-2"))
            .child(
                div()
                    .absolute()
                    .top(px(-1.))
                    .bottom(px(-1.))
                    .left(px(-1.))
                    .w(px(CARD_BAND))
                    .rounded_l(px(radius::MD))
                    .bg(band),
            )
            .when(s(p, "title").is_some() || s(p, "subtitle").is_some(), |d| {
                d.child(
                    div()
                        .flex()
                        .flex_col()
                        .when_some(s(p, "title"), |d, title| {
                            d.child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(px(space::SM))
                                    .when(tone != "neutral", |d| {
                                        d.child(
                                            div().flex_none().size(px(DOT)).rounded_full().bg(edge),
                                        )
                                    })
                                    .child(
                                        div().font_weight(weight::STRONG).child(title.to_string()),
                                    ),
                            )
                        })
                        .when_some(s(p, "subtitle"), |d, sub| {
                            d.child(txt(sub.to_string(), size::SMALL, weight::REGULAR, ink3))
                        }),
                )
            })
            .children(children)
            .into_any_element()
    }

    fn tabs(&mut self, p: &Value, id: &str, kids: &Children<'a>) -> AnyElement {
        let labels: Vec<String> = arr(p, "labels")
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_string)
            .collect();
        let active = self.ui.tabs.get(id).copied().unwrap_or(0);
        // Every panel renders, so a Form spanning tabs keeps its fields; only the active one shows.
        let panels = self.children(kids);
        let pal = self.pal;
        let strip = div()
            .id(eid(format!("{id}-tablist")))
            .flex()
            .gap(px(space::XL))
            .border_b_1()
            .border_color(pal.c("line"))
            .overflow_x_scroll()
            .children(labels.iter().enumerate().map(|(i, label)| {
                let on = i == active;
                let id = id.to_string();
                // The selected tab is 700 in ink over its underline; the rest recede.
                focusable(div().id(eid(format!("{id}-tab-{i}"))), pal, true)
                    .flex()
                    .items_center()
                    .h(px(control::MD))
                    .border_b_2()
                    .border_color(if on {
                        pal.c("ink")
                    } else {
                        gpui::transparent_black()
                    })
                    .text_size(px(size::UI))
                    .text_color(if on { pal.c("ink") } else { pal.c("ink-3") })
                    .font_weight(if on { weight::STRONG } else { weight::REGULAR })
                    .whitespace_nowrap()
                    .cursor_pointer()
                    .when(!on, |d| d.hover(|s| s.text_color(pal.c("ink"))))
                    .child(label.clone())
                    .on_click(self.cx.listener(move |this, _, _, cx| {
                        this.ui.tabs.insert(id.clone(), i);
                        cx.notify();
                    }))
            }));
        let shown = panels.into_iter().nth(active);
        div()
            .flex()
            .flex_col()
            .child(strip)
            .when_some(shown, |d, panel| {
                d.child(div().pt(px(space::LG)).child(panel))
            })
            .into_any_element()
    }

    /// A `<details>`: `open` is only where it starts.
    pub fn details_open(&mut self, id: &str, initial: bool) -> bool {
        let entry = self
            .ui
            .open
            .entry(id.to_string())
            .or_insert((initial, initial));
        if entry.0 != initial {
            *entry = (initial, initial);
        }
        entry.1
    }

    /// A fold's label: `SMALL` 13/400 in `ink-3` behind its chevron.
    pub fn summary(
        &mut self,
        id: &str,
        label: SharedString,
        open: bool,
    ) -> impl IntoElement + use<> {
        let ink3 = self.pal.c("ink-3");
        let ink = self.pal.c("ink");
        let key = id.to_string();
        div().flex().child(
            focusable(div().id(eid(format!("{id}-summary"))), self.pal, false)
                .flex()
                .items_center()
                .gap(px(space::XS))
                .rounded(px(radius::SM))
                .cursor_pointer()
                .text_size(px(size::SMALL))
                .line_height(px(leading::SMALL))
                .text_color(ink3)
                .hover(move |s| s.text_color(ink))
                .child(ui::icon(
                    if open {
                        "chevron-down"
                    } else {
                        "chevron-right"
                    },
                    icon_size::INLINE,
                    ink3,
                ))
                .child(label)
                .on_click(self.cx.listener(move |this, _, _, cx| {
                    if let Some(e) = this.ui.open.get_mut(&key) {
                        e.1 = !e.1;
                    }
                    cx.notify();
                })),
        )
    }

    fn disclosure(&mut self, p: &Value, id: &str, kids: &Children<'a>) -> AnyElement {
        let open = self.details_open(id, flag(p, "open"));
        // Closed details keep their content mounted.
        let children = self.children(kids);
        let summary = self.summary(
            id,
            s(p, "label").unwrap_or_default().to_string().into(),
            open,
        );
        div()
            .flex()
            .flex_col()
            .child(summary)
            .when(open, |d| {
                d.child(div().pt(px(space::MD)).flex().flex_col().children(children))
            })
            .into_any_element()
    }

    // ------------------------------------------------------------------ text

    /// Headings stay on the scale and at 700: 900 is the pane's title and names (T2).
    fn heading(&mut self, p: &Value) -> AnyElement {
        let size = match n(p, "level").map(|l| l as i64) {
            Some(1) => size::TITLE,
            Some(3) => size::UI,
            _ => size::BODY,
        };
        txt(
            s(p, "text").unwrap_or_default().to_string(),
            size,
            weight::STRONG,
            self.pal.c("ink"),
        )
        .into_any_element()
    }

    fn text(&mut self, p: &Value) -> AnyElement {
        let color = match s(p, "tone") {
            Some("muted") => self.pal.c("ink-3"),
            Some(t @ ("success" | "warning" | "danger")) => self.pal.tone(t).1,
            _ => self.pal.c("ink"),
        };
        div()
            .text_color(color)
            .map(|d| match s(p, "size") {
                Some("sm") => d.text_size(px(size::SMALL)).line_height(px(leading::SMALL)),
                Some("lg") => d.text_size(px(size::TITLE)).line_height(px(leading::TITLE)),
                _ => d,
            })
            .when(flag(p, "strong"), |d| d.font_weight(weight::STRONG))
            .child(s(p, "text").unwrap_or_default().to_string())
            .into_any_element()
    }

    fn markdown(&mut self, p: &Value, id: &str) -> AnyElement {
        let source = s(p, "source").unwrap_or_default().to_string();
        super::md::view(self, &source, id)
    }

    fn callout(&mut self, p: &Value) -> AnyElement {
        let tone = s(p, "tone").unwrap_or("neutral");
        let (_, ink, bg) = self.pal.tone(tone);
        let icon = match tone {
            "success" => "check",
            "warning" | "danger" => "triangle-alert",
            _ => "info",
        };
        div()
            .flex()
            .gap(px(space::MD))
            .px(px(space::LG))
            .py(px(space::MD))
            .rounded(px(radius::MD))
            .bg(bg)
            .text_color(ink)
            .text_size(px(size::UI))
            .line_height(px(leading::UI))
            .child(lead(ui::icon(icon, icon_size::ROW, ink), leading::UI))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .min_w_0()
                    .when_some(s(p, "title"), |d, title| {
                        d.child(div().font_weight(weight::STRONG).child(title.to_string()))
                    })
                    .child(s(p, "text").unwrap_or_default().to_string()),
            )
            .into_any_element()
    }

    fn quote(&mut self, p: &Value) -> AnyElement {
        div()
            .flex()
            .flex_col()
            .pl(px(space::LG))
            .border_l_1()
            .border_color(self.pal.c("line-strong"))
            .text_color(self.pal.c("ink-2"))
            .child(s(p, "text").unwrap_or_default().to_string())
            .when_some(s(p, "cite"), |d, cite| {
                d.child(txt(
                    cite.to_string(),
                    size::SMALL,
                    weight::REGULAR,
                    self.pal.c("ink-3"),
                ))
            })
            .into_any_element()
    }

    /// A state word: a chip, never a pill (R2).
    fn badge(&mut self, p: &Value) -> AnyElement {
        let (_, ink, bg) = self.pal.tone(s(p, "tone").unwrap_or("neutral"));
        div()
            .flex()
            .child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .h(px(leading::UI))
                    .px(px(space::SM))
                    .rounded(px(radius::SM))
                    .bg(bg)
                    .text_color(ink)
                    .text_size(px(size::META))
                    .line_height(px(leading::META))
                    .font_weight(weight::STRONG)
                    .whitespace_nowrap()
                    .child(s(p, "text").unwrap_or_default().to_string()),
            )
            .into_any_element()
    }

    fn key_value(&mut self, p: &Value) -> AnyElement {
        let ink3 = self.pal.c("ink-3");
        div()
            .flex()
            .flex_col()
            .gap(px(space::XS))
            .text_size(px(size::UI))
            .line_height(px(leading::UI))
            .children(arr(p, "items").iter().map(|item| {
                div()
                    .flex()
                    .gap(px(space::LG))
                    .child(
                        div()
                            .w(relative(0.3))
                            .min_w(px(KEY_MIN_W))
                            .flex_none()
                            .text_color(ink3)
                            .child(s(item, "label").unwrap_or_default().to_string()),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .child(s(item, "value").unwrap_or_default().to_string()),
                    )
            }))
            .into_any_element()
    }

    fn list(&mut self, p: &Value) -> AnyElement {
        let ordered = flag(p, "ordered");
        let ink3 = self.pal.c("ink-3");
        div()
            .flex()
            .flex_col()
            .gap(px(space::HAIR))
            .children(arr(p, "items").iter().enumerate().map(|(i, item)| {
                let marker = if ordered {
                    format!("{}.", i + 1)
                } else {
                    "•".to_string()
                };
                div()
                    .flex()
                    // A fixed box: "9." and "10." end on the same line (T8).
                    .child(
                        div()
                            .w(px(space::SECTION))
                            .flex_none()
                            .flex()
                            .justify_end()
                            .pr(px(space::SM))
                            .text_color(ink3)
                            .child(marker),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .child(item.as_str().unwrap_or_default().to_string()),
                    )
            }))
            .into_any_element()
    }

    fn link(&mut self, p: &Value, id: &str) -> AnyElement {
        let href = s(p, "href").unwrap_or_default().to_string();
        let color = self.pal.c("link");
        div()
            .flex()
            .child(
                focusable(div().id(eid(format!("{id}-link"))), self.pal, false)
                    .rounded(px(radius::SM))
                    .text_color(color)
                    .cursor_pointer()
                    .hover(|s| s.underline())
                    .child(s(p, "label").unwrap_or_default().to_string())
                    // Outside the app, in the operator's browser.
                    .on_click(move |_, _, cx| cx.open_url(&href)),
            )
            .into_any_element()
    }

    /// The block Code, Diff and Terminal share: a quiet fill instead of a
    /// second border inside the view's frame (C4), a hairline under its head.
    pub fn code_box(&self, head: Option<AnyElement>, body: AnyElement) -> AnyElement {
        div()
            .flex()
            .flex_col()
            .rounded(px(radius::MD))
            .bg(self.pal.c("surface-2"))
            .overflow_hidden()
            .when_some(head, |d, head| {
                d.child(
                    div()
                        .flex()
                        .justify_between()
                        .gap(px(space::LG))
                        .px(px(space::LG))
                        .py(px(space::SM))
                        .border_b_1()
                        .border_color(self.pal.c("line"))
                        .font_family(ui::MONO)
                        .text_size(px(size::META))
                        .line_height(px(leading::META))
                        .text_color(self.pal.c("ink-3"))
                        .child(head),
                )
            })
            .child(body)
            .into_any_element()
    }

    fn code(&mut self, p: &Value, id: &str) -> AnyElement {
        let head = s(p, "filename")
            .or(s(p, "language"))
            .map(|h| div().child(h.to_string()).into_any_element());
        let body = div()
            .id(eid(format!("{id}-code")))
            .max_h(px(CODE_MAX_H))
            .overflow_scroll()
            .px(px(space::LG))
            .py(px(space::MD))
            .font_family(ui::MONO)
            .text_size(px(size::SMALL))
            .line_height(px(leading::UI))
            .whitespace_nowrap()
            .child(s(p, "code").unwrap_or_default().to_string())
            .into_any_element();
        self.code_box(head, body)
    }

    // ------------------------------------------------------------------ data

    /// One figure: its label and hint recede, the number carries the block.
    fn metric(&mut self, p: &Value) -> AnyElement {
        let ink = match s(p, "deltaTone") {
            Some(t @ ("success" | "danger")) => self.pal.tone(t).1,
            _ => self.pal.c("ink-3"),
        };
        div()
            .flex()
            .flex_col()
            .min_w_0()
            .child(
                txt(
                    s(p, "label").unwrap_or_default().to_string(),
                    size::SMALL,
                    weight::REGULAR,
                    self.pal.c("ink-3"),
                )
                .truncate(),
            )
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_baseline()
                    .gap_x(px(space::SM))
                    .child(txt(
                        s(p, "value").unwrap_or_default().to_string(),
                        size::TITLE,
                        weight::STRONG,
                        self.pal.c("ink"),
                    ))
                    .when_some(s(p, "delta"), |d, delta| {
                        d.child(txt(delta.to_string(), size::SMALL, weight::REGULAR, ink))
                    }),
            )
            .when_some(s(p, "hint"), |d, hint| {
                d.child(txt(
                    hint.to_string(),
                    size::SMALL,
                    weight::REGULAR,
                    self.pal.c("ink-3"),
                ))
            })
            .into_any_element()
    }

    /// "Show N more" (`.hb-att-more`): a link at 400.
    pub fn more_button(
        &mut self,
        id: String,
        label: SharedString,
        on: impl Fn(&mut HuginnView) + 'static,
    ) -> AnyElement {
        div()
            .flex()
            .child(
                focusable(div().id(eid(id)), self.pal, false)
                    .mt(px(space::XS))
                    .rounded(px(radius::SM))
                    .text_size(px(size::SMALL))
                    .line_height(px(leading::SMALL))
                    .text_color(self.pal.c("link"))
                    .cursor_pointer()
                    .hover(|s| s.underline())
                    .child(label)
                    .on_click(self.cx.listener(move |this, _, _, cx| {
                        on(this);
                        cx.notify();
                    })),
            )
            .into_any_element()
    }

    /// Column widths as an HTML table with `width: 100%` lays them out.
    pub fn column_widths(
        &mut self,
        id: &str,
        head: &[String],
        rows: &[Vec<String>],
        width: f32,
    ) -> Vec<f32> {
        let fingerprint = {
            use std::hash::{Hash, Hasher};
            let mut h = std::collections::hash_map::DefaultHasher::new();
            head.hash(&mut h);
            rows.hash(&mut h);
            h.finish()
        };
        let cached = self
            .ui
            .tables
            .get(id)
            .and_then(|t| t.measured.clone())
            .filter(|m| m.0 == fingerprint);
        let (max, min) = match cached {
            Some((_, max, min)) => (max, min),
            None => {
                let cols = head.len();
                let mut max = vec![0f32; cols];
                let mut min = vec![0f32; cols];
                for (c, h) in head.iter().enumerate() {
                    // Room for the sort arrow, so sorting never moves a column.
                    let w = self.measure(&format!("{h} ↓"), size::META, weight::STRONG, ui::SANS);
                    max[c] = max[c].max(w);
                    min[c] = min[c].max(w);
                }
                for row in rows {
                    for (c, cell) in row.iter().enumerate().take(cols) {
                        max[c] =
                            max[c].max(self.measure(cell, size::UI, weight::REGULAR, ui::SANS));
                        let longest = cell
                            .split(' ')
                            .max_by_key(|w| w.chars().count())
                            .unwrap_or_default();
                        min[c] =
                            min[c].max(self.measure(longest, size::UI, weight::REGULAR, ui::SANS));
                    }
                }
                let pad = 2. * space::MD;
                let max: Vec<f32> = max.iter().map(|w| w.ceil() + pad).collect();
                let min: Vec<f32> = min.iter().map(|w| w.ceil() + pad).collect();
                self.ui.tables.entry(id.to_string()).or_default().measured =
                    Some((fingerprint, max.clone(), min.clone()));
                (max, min)
            }
        };
        let sum_max: f32 = max.iter().sum();
        let sum_min: f32 = min.iter().sum();
        if sum_max <= width {
            let extra = width - sum_max;
            max.iter()
                .map(|w| {
                    w + if sum_max > 0.0 {
                        extra * w / sum_max
                    } else {
                        0.0
                    }
                })
                .collect()
        } else if sum_min >= width {
            min
        } else {
            let room = width - sum_min;
            let spread = sum_max - sum_min;
            min.iter()
                .zip(&max)
                .map(|(lo, hi)| {
                    lo + if spread > 0.0 {
                        room * (hi - lo) / spread
                    } else {
                        0.0
                    }
                })
                .collect()
        }
    }

    /// A plain table: a `META` 12/700 header in `ink-3`, 32 px rows of
    /// `UI` 14, hairlines between rows and none under the last.
    pub fn grid_table(
        &mut self,
        id: &str,
        head: Vec<AnyElement>,
        head_text: &[String],
        rows: &[Vec<String>],
        measure_rows: &[Vec<String>],
        end: &[bool],
    ) -> AnyElement {
        let (width, probe) = self.width_probe(&format!("{id}#table"), 640.);
        let widths = self.column_widths(id, head_text, measure_rows, width);
        let total: f32 = widths.iter().sum();
        let pal = self.pal;
        let header = div()
            .flex()
            .children(head.into_iter().enumerate().map(|(c, h)| {
                div()
                    .w(px(widths[c]))
                    .flex_none()
                    .px(px(space::MD))
                    .py(px(space::SM))
                    .border_b_1()
                    .border_color(pal.c("line-strong"))
                    .text_size(px(size::META))
                    .line_height(px(leading::META))
                    .font_weight(weight::STRONG)
                    .text_color(pal.c("ink-3"))
                    .whitespace_nowrap()
                    .flex()
                    .when(end.get(c).copied().unwrap_or(false), |d| d.justify_end())
                    .child(h)
            }));
        let last = rows.len().saturating_sub(1);
        let body = rows.iter().enumerate().map(|(r, row)| {
            div()
                .flex()
                .children(row.iter().enumerate().map(|(c, cell)| {
                    div()
                        .w(px(widths.get(c).copied().unwrap_or(0.)))
                        .flex_none()
                        .px(px(space::MD))
                        .py(px(space::SM))
                        .when(r < last, |d| d.border_b_1().border_color(pal.c("line")))
                        .when(end.get(c).copied().unwrap_or(false), |d| {
                            d.flex().justify_end()
                        })
                        .child(cell.clone())
                }))
        });
        div()
            .relative()
            .w_full()
            .child(probe)
            .child(
                div()
                    .id(eid(format!("{id}-scroll")))
                    .overflow_x_scroll()
                    .w_full()
                    .child(
                        div()
                            .w(px(total))
                            .text_size(px(size::UI))
                            .line_height(px(leading::UI))
                            .flex()
                            .flex_col()
                            .child(header)
                            .children(body),
                    ),
            )
            .into_any_element()
    }

    fn table(&mut self, p: &Value, id: &str) -> AnyElement {
        let columns = arr(p, "columns");
        let head_text: Vec<String> = columns
            .iter()
            .map(|c| s(c, "label").unwrap_or_default().to_string())
            .collect();
        let raw: Vec<&Vec<Value>> = arr(p, "rows").iter().filter_map(Value::as_array).collect();
        // A column of numbers ends on one edge whether or not the view asked (T8).
        let end: Vec<bool> = columns
            .iter()
            .enumerate()
            .map(|(c, col)| {
                s(col, "align") == Some("end")
                    || (raw.iter().any(|r| r.get(c).is_some_and(Value::is_number))
                        && raw
                            .iter()
                            .all(|r| r.get(c).is_none_or(|v| v.is_number() || v.is_null())))
            })
            .collect();
        let state = self.ui.tables.get(id).cloned().unwrap_or_default();
        let mut ordered = raw.clone();
        if let Some((column, down)) = state.sort {
            ordered.sort_by(|a, b| {
                let o = compare_cells(a.get(column), b.get(column));
                if down { o.reverse() } else { o }
            });
        }
        let total = ordered.len();
        let shown = if state.all { total } else { total.min(50) };
        let cell_text = |v: &Value| match v {
            Value::Number(n) => format_plain(n.as_f64().unwrap_or(0.0), self.locale),
            Value::String(s) => collapse(s),
            _ => String::new(),
        };
        let all_rows: Vec<Vec<String>> = raw
            .iter()
            .map(|r| r.iter().map(cell_text).collect())
            .collect();
        let rows: Vec<Vec<String>> = ordered[..shown]
            .iter()
            .map(|r| r.iter().map(cell_text).collect())
            .collect();
        let sortable = flag(p, "sortable");
        let head: Vec<AnyElement> = head_text
            .iter()
            .enumerate()
            .map(|(c, label)| {
                let arrow = match state.sort {
                    Some((col, down)) if col == c => {
                        if down {
                            " ↓"
                        } else {
                            " ↑"
                        }
                    }
                    _ => "",
                };
                let text = format!("{label}{arrow}");
                if sortable {
                    let key = id.to_string();
                    focusable(div().id(eid(format!("{id}-sort-{c}"))), self.pal, false)
                        .rounded(px(radius::SM))
                        .cursor_pointer()
                        .hover(|s| s.text_color(self.pal.c("ink")))
                        .child(text)
                        .on_click(self.cx.listener(move |this, _, _, cx| {
                            let t = this.ui.tables.entry(key.clone()).or_default();
                            t.sort = match t.sort {
                                Some((col, down)) if col == c => Some((c, !down)),
                                _ => Some((c, false)),
                            };
                            cx.notify();
                        }))
                        .into_any_element()
                } else {
                    div().child(text).into_any_element()
                }
            })
            .collect();
        // Widths come from every row, so sorting or showing more never moves a column.
        let grid = self.grid_table(id, head, &head_text, &rows, &all_rows, &end);
        let caption = s(p, "caption").map(|c| {
            txt(
                c.to_string(),
                size::SMALL,
                weight::REGULAR,
                self.pal.c("ink-3"),
            )
            .pb(px(space::XS))
        });
        let key = id.to_string();
        let more = (total > shown).then(|| {
            let label = self.trv("showRemaining", &[("count", &(total - shown).to_string())]);
            self.more_button(format!("{id}-more"), label, move |this| {
                this.ui.tables.entry(key.clone()).or_default().all = true;
            })
        });
        div()
            .flex()
            .flex_col()
            .children(caption)
            .child(grid)
            .children(more)
            .into_any_element()
    }

    /// How far along: the amount is not a state, so the bar is neutral
    /// unless the view gives it one (C3).
    fn progress(&mut self, p: &Value) -> AnyElement {
        let top = n(p, "max").unwrap_or(100.0);
        let value = n(p, "value").unwrap_or(0.0);
        let pct = (value / top * 100.0).clamp(0.0, 100.0);
        let fill = match s(p, "tone") {
            Some(t @ ("success" | "warning" | "danger")) => self.pal.tone(t).0,
            _ => self.pal.c("ink-3"),
        };
        div()
            .flex()
            .flex_col()
            .gap(px(space::XS))
            .text_size(px(size::UI))
            .line_height(px(leading::UI))
            .child(
                div()
                    .flex()
                    .justify_between()
                    .gap(px(space::LG))
                    .child(
                        div()
                            .min_w_0()
                            .truncate()
                            .child(s(p, "label").unwrap_or_default().to_string()),
                    )
                    .child(
                        div()
                            .flex_none()
                            .text_color(self.pal.c("ink-2"))
                            .child(self.percent(pct.round())),
                    ),
            )
            .child(
                div()
                    .h(px(space::SM))
                    .rounded(px(radius::SM))
                    .bg(self.pal.c("chip"))
                    .overflow_hidden()
                    .child(
                        div()
                            .h_full()
                            .rounded(px(radius::SM))
                            .bg(fill)
                            .w(relative(pct as f32 / 100.0)),
                    ),
            )
            .when_some(s(p, "hint"), |d, hint| {
                d.child(txt(
                    hint.to_string(),
                    size::SMALL,
                    weight::REGULAR,
                    self.pal.c("ink-3"),
                ))
            })
            .into_any_element()
    }

    fn timeline(&mut self, p: &Value) -> AnyElement {
        let pal = self.pal;
        div()
            .flex()
            .flex_col()
            .gap(px(space::MD))
            .pl(px(space::LG))
            .border_l_1()
            .border_color(pal.c("line-strong"))
            .text_size(px(size::UI))
            .line_height(px(leading::UI))
            .children(arr(p, "items").iter().map(|item| {
                let (edge, _, _) = pal.tone(s(item, "tone").unwrap_or("neutral"));
                // The dot sits on the rule, centred on the item's first line.
                let first = if s(item, "time").is_some() {
                    leading::META
                } else {
                    leading::UI
                };
                div()
                    .relative()
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .absolute()
                            .left(px(-(space::LG + 0.5 + DOT / 2.)))
                            .top(px((first - DOT) / 2.))
                            .size(px(DOT))
                            .rounded_full()
                            .bg(edge),
                    )
                    .when_some(s(item, "time"), |d, time| {
                        d.child(txt(
                            time.to_string(),
                            size::META,
                            weight::REGULAR,
                            pal.c("ink-3"),
                        ))
                    })
                    .child(s(item, "title").unwrap_or_default().to_string())
                    .when_some(s(item, "detail"), |d, detail| {
                        d.child(txt(
                            detail.to_string(),
                            size::SMALL,
                            weight::REGULAR,
                            pal.c("ink-3"),
                        ))
                    })
            }))
            .into_any_element()
    }

    /// Done is green, failed red, a warning amber; the step in hand is
    /// merely current, so it is 700 in ink with no state colour (C3).
    fn steps(&mut self, p: &Value, checklist: bool) -> AnyElement {
        let pal = self.pal;
        let rows: Vec<AnyElement> = arr(p, "items")
            .iter()
            .map(|item| {
                let state = s(item, "state").unwrap_or_default();
                let (icon, color, word) = match (checklist, state) {
                    (false, "done") => ("check", pal.c("green-ink"), "stepDone"),
                    (false, "current") => ("circle-dot", pal.c("ink"), "stepCurrent"),
                    (false, "failed") => ("x", pal.c("red-ink"), "stepFailed"),
                    (false, _) => ("circle", pal.c("line-strong"), "stepTodo"),
                    (true, "pass") => ("check", pal.c("green-ink"), "checkPass"),
                    (true, "fail") => ("x", pal.c("red-ink"), "checkFail"),
                    (true, "warn") => ("triangle-alert", pal.c("amber-ink"), "checkWarn"),
                    (true, _) => ("minus", pal.c("ink-4"), "checkSkip"),
                };
                let current = !checklist && state == "current";
                let quiet = matches!((checklist, state), (false, "todo") | (true, "skip"));
                let word = self.tr(word);
                div()
                    .flex()
                    .gap(px(space::MD))
                    .child(lead(ui::icon(icon, icon_size::ROW, color), leading::UI))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .min_w_0()
                            .child(
                                div()
                                    .flex()
                                    .flex_wrap()
                                    .items_baseline()
                                    .gap_x(px(space::SM))
                                    .child(
                                        div()
                                            .when(current, |d| d.font_weight(weight::STRONG))
                                            .when(quiet, |d| d.text_color(pal.c("ink-3")))
                                            .child(
                                                s(item, "label").unwrap_or_default().to_string(),
                                            ),
                                    )
                                    .child(txt(word, size::SMALL, weight::REGULAR, pal.c("ink-3"))),
                            )
                            .when_some(s(item, "detail"), |d, detail| {
                                d.child(txt(
                                    detail.to_string(),
                                    size::SMALL,
                                    weight::REGULAR,
                                    pal.c("ink-3"),
                                ))
                            }),
                    )
                    .into_any_element()
            })
            .collect();
        div()
            .flex()
            .flex_col()
            .gap(px(space::SM))
            .text_size(px(size::UI))
            .line_height(px(leading::UI))
            .children(rows)
            .into_any_element()
    }

    // ------------------------------------------------------------------ work

    fn diff(&mut self, p: &Value, id: &str) -> AnyElement {
        let source = s(p, "source")
            .unwrap_or_default()
            .replace("\r\n", "\n")
            .replace('\r', "\n");
        let lines: Vec<&str> = source.split('\n').collect();
        let full = self.ui.full.contains(id);
        let shown = if full {
            lines.len()
        } else {
            lines.len().min(60)
        };
        let pal = self.pal;
        let body = div()
            .id(eid(format!("{id}-diff")))
            .max_h(px(CODE_MAX_H))
            .overflow_y_scroll()
            .py(px(space::XS))
            .font_family(ui::MONO)
            .text_size(px(size::SMALL))
            .line_height(px(leading::UI))
            .children(lines[..shown].iter().map(|line| {
                let (bg, ink, bold) = if line.starts_with("+++")
                    || line.starts_with("---")
                    || line.starts_with("diff ")
                    || line.starts_with("index ")
                {
                    (None, pal.c("ink-2"), true)
                } else if line.starts_with("@@") {
                    (Some(pal.c("tint")), pal.c("link"), false)
                } else if line.starts_with('+') {
                    (Some(pal.c("green-bg")), pal.c("green-ink"), false)
                } else if line.starts_with('-') {
                    (Some(pal.c("red-bg")), pal.c("red-ink"), false)
                } else {
                    (None, pal.c("ink-2"), false)
                };
                div()
                    .px(px(space::LG))
                    .when_some(bg, |d, bg| d.bg(bg))
                    .text_color(ink)
                    .when(bold, |d| d.font_weight(weight::STRONG))
                    .child(if line.is_empty() {
                        " ".to_string()
                    } else {
                        (*line).to_string()
                    })
            }));
        let more_count = lines.len() - shown;
        let key = id.to_string();
        let more = (more_count > 0).then(|| {
            let label = tv(
                self.cx,
                "attachments.showFull",
                &[("n", &more_count.to_string())],
            );
            self.more_button(format!("{id}-more"), label, move |this| {
                this.ui.full.insert(key.clone());
            })
        });
        let head = s(p, "filename").map(|f| div().child(f.to_string()).into_any_element());
        let body = div()
            .flex()
            .flex_col()
            .child(body)
            .children(more)
            .into_any_element();
        self.code_box(head, body)
    }

    fn file_tree(&mut self, p: &Value) -> AnyElement {
        let pal = self.pal;
        let rows: Vec<AnyElement> = arr(p, "entries")
            .iter()
            .map(|entry| {
                let status = s(entry, "status");
                let (word, color) = match status {
                    Some("added") => (Some("fileAdded"), pal.c("green-ink")),
                    Some("deleted") => (Some("fileDeleted"), pal.c("red-ink")),
                    Some("modified") => (Some("fileModified"), pal.c("ink-3")),
                    Some("renamed") => (Some("fileRenamed"), pal.c("ink-3")),
                    _ => (None, pal.c("ink-3")),
                };
                let word = word.map(|w| self.tr(w));
                div()
                    .flex()
                    .flex_wrap()
                    .items_baseline()
                    .gap(px(space::MD))
                    .child(
                        div()
                            .min_w_0()
                            .when(status == Some("deleted"), |d| d.line_through())
                            .child(s(entry, "path").unwrap_or_default().to_string()),
                    )
                    .when_some(word, |d, w| {
                        d.child(txt(w, size::META, weight::REGULAR, color).font_family(ui::SANS))
                    })
                    .when_some(s(entry, "note"), |d, note| {
                        d.child(
                            txt(
                                note.to_string(),
                                size::META,
                                weight::REGULAR,
                                pal.c("ink-3"),
                            )
                            .font_family(ui::SANS),
                        )
                    })
                    .into_any_element()
            })
            .collect();
        div()
            .flex()
            .flex_col()
            .gap(px(space::HAIR))
            .font_family(ui::MONO)
            .text_size(px(size::SMALL))
            .line_height(px(leading::UI))
            .children(rows)
            .into_any_element()
    }

    fn terminal(&mut self, p: &Value, id: &str) -> AnyElement {
        let command = s(p, "command");
        let exit = n(p, "exitCode").map(|c| c as i64);
        let pal = self.pal;
        let head = (command.is_some() || exit.is_some()).then(|| {
            let exit_label = exit.map(|c| self.trv("exitCode", &[("code", &c.to_string())]));
            div()
                .flex()
                .flex_1()
                .justify_between()
                .gap(px(space::LG))
                .font_family(ui::MONO)
                .child(div().child(command.map(|c| format!("$ {c}")).unwrap_or_default()))
                .when_some(exit_label, |d, label| {
                    let ok = exit == Some(0);
                    d.child(txt(
                        label,
                        size::META,
                        weight::STRONG,
                        if ok {
                            pal.c("green-ink")
                        } else {
                            pal.c("red-ink")
                        },
                    ))
                })
                .into_any_element()
        });
        let output = s(p, "output")
            .unwrap_or_default()
            .replace("\r\n", "\n")
            .replace('\r', "\n");
        let trimmed = output.strip_suffix('\n').unwrap_or(&output);
        let lines: Vec<&str> = trimmed.split('\n').collect();
        let full = self.ui.full.contains(id);
        let more_count = lines.len().saturating_sub(24);
        let text = if full {
            output.clone()
        } else {
            lines[..lines.len().min(24)].join("\n")
        };
        let body = div()
            .id(eid(format!("{id}-log")))
            .max_h(px(CODE_MAX_H))
            .overflow_y_scroll()
            .px(px(space::LG))
            .py(px(space::MD))
            .text_color(pal.c("ink"))
            .font_family(ui::MONO)
            .text_size(px(size::SMALL))
            .line_height(px(leading::UI))
            .child(text);
        let key = id.to_string();
        let more = (!full && more_count > 0).then(|| {
            let label = tv(
                self.cx,
                "attachments.showFull",
                &[("n", &more_count.to_string())],
            );
            self.more_button(format!("{id}-more"), label, move |this| {
                this.ui.full.insert(key.clone());
            })
        });
        let body = div()
            .flex()
            .flex_col()
            .child(body)
            .children(more)
            .into_any_element();
        self.code_box(head, body)
    }

    /// The message's image attachment with this name.
    pub fn image_file(&self, name: &str) -> Option<&'a Attachment> {
        self.files
            .iter()
            .find(|f| f.name == name && f.kind == "image")
    }

    pub fn missing(&self, file: &str) -> AnyElement {
        self.error_line(self.trv("missingAttachment", &[("file", file)]))
    }

    /// An attachment's picture at a width, or a quiet box while it loads.
    pub fn picture(&self, file: &Attachment, max_w: f32, max_h: Option<f32>) -> Div {
        let pal = self.pal;
        match self.ui.images.get(&file.id) {
            Some(super::ImageLoad::Ready(image, w, h)) => {
                let (w, h) = (*w as f32, *h as f32);
                let mut scale = (max_w / w).min(1.0);
                if let Some(mh) = max_h {
                    scale = scale.min(mh / h);
                }
                div().child(
                    gpui::img(image.clone())
                        .w(px((w * scale).max(1.0)))
                        .h(px((h * scale).max(1.0)))
                        .rounded(px(radius::BASE))
                        .border_1()
                        .border_color(pal.c("line"))
                        .bg(pal.c("surface")),
                )
            }
            Some(super::ImageLoad::Failed) => div().child(self.missing(&file.name)),
            _ => div()
                .w(px(max_w.min(PLACEHOLDER_W)))
                .h(px(PLACEHOLDER_H))
                .rounded(px(radius::BASE))
                .border_1()
                .border_color(pal.c("line")),
        }
    }

    fn image(&mut self, p: &Value) -> AnyElement {
        let name = s(p, "file").unwrap_or_default();
        let Some(file) = self.image_file(name) else {
            return self.missing(name);
        };
        let (width, probe) = self.width_probe(&format!("image:{}", file.id), 640.);
        let picture = self.picture(file, width, Some(IMAGE_MAX_H));
        div()
            .relative()
            .flex()
            .flex_col()
            .child(probe)
            .child(picture)
            .when_some(s(p, "caption"), |d, c| {
                d.child(
                    txt(
                        c.to_string(),
                        size::SMALL,
                        weight::REGULAR,
                        self.pal.c("ink-3"),
                    )
                    .pt(px(space::XS)),
                )
            })
            .into_any_element()
    }
}

/// An element's children and how to render them.
pub(super) struct Children<'a> {
    pub el: &'a Element,
    pub scope: Scope<'a>,
    pub filter: Option<Value>,
}

/// Fold text for a locale-aware, numeric comparison (`localeCompare(…, "es",
/// {numeric: true})`): case and accents matter only to break ties.
fn fold(c: char) -> char {
    let lower = c.to_lowercase().next().unwrap_or(c);
    match lower {
        'á' | 'à' | 'â' | 'ä' | 'ã' | 'å' => 'a',
        'é' | 'è' | 'ê' | 'ë' => 'e',
        'í' | 'ì' | 'î' | 'ï' => 'i',
        'ó' | 'ò' | 'ô' | 'ö' | 'õ' => 'o',
        'ú' | 'ù' | 'û' | 'ü' => 'u',
        'ç' => 'c',
        other => other,
    }
}

fn natural(a: &str, b: &str) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    let (mut x, mut y) = (a.chars().peekable(), b.chars().peekable());
    loop {
        match (x.peek().copied(), y.peek().copied()) {
            (None, None) => return Ordering::Equal,
            (None, _) => return Ordering::Less,
            (_, None) => return Ordering::Greater,
            (Some(p), Some(q)) if p.is_ascii_digit() && q.is_ascii_digit() => {
                let mut da = String::new();
                while let Some(c) = x.peek().copied().filter(char::is_ascii_digit) {
                    da.push(c);
                    x.next();
                }
                let mut db = String::new();
                while let Some(c) = y.peek().copied().filter(char::is_ascii_digit) {
                    db.push(c);
                    y.next();
                }
                let (ta, tb) = (da.trim_start_matches('0'), db.trim_start_matches('0'));
                let o = ta.len().cmp(&tb.len()).then_with(|| ta.cmp(tb));
                if o != Ordering::Equal {
                    return o;
                }
            }
            (Some(p), Some(q)) => {
                let o = fold(p).cmp(&fold(q));
                if o != Ordering::Equal {
                    return o;
                }
                x.next();
                y.next();
            }
        }
    }
}

pub(super) fn locale_compare(a: &str, b: &str) -> std::cmp::Ordering {
    natural(a, b).then_with(|| {
        // Lowercase before uppercase, plain before accented, as ICU's tertiary level.
        let key = |s: &str| {
            s.chars()
                .map(|c| (c != fold(c), c.is_uppercase()))
                .collect::<Vec<_>>()
        };
        key(a).cmp(&key(b))
    })
}

/// The Table comparator: numbers by value, the rest by `localeCompare`,
/// nulls last when ascending.
pub(super) fn compare_cells(a: Option<&Value>, b: Option<&Value>) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    let null = |v: Option<&Value>| v.is_none_or(Value::is_null);
    match (null(a), null(b)) {
        (true, true) => return Ordering::Equal,
        (true, false) => return Ordering::Greater,
        (false, true) => return Ordering::Less,
        _ => {}
    }
    let (a, b) = (a.unwrap_or(&Value::Null), b.unwrap_or(&Value::Null));
    if let (Some(x), Some(y)) = (a.as_f64(), b.as_f64())
        && a.is_number()
        && b.is_number()
    {
        return x.partial_cmp(&y).unwrap_or(Ordering::Equal);
    }
    locale_compare(&expr::js_string(a), &expr::js_string(b))
}

/// Read the props of an element by key (for the answer words).
pub(super) fn props_of<'s>(spec: &'s Spec, key: &str) -> Option<&'s Map<String, Value>> {
    spec.elements.get(key).map(|e| &e.props)
}

pub(super) type ArcImage = Arc<gpui::Image>;

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::cmp::Ordering;

    #[test]
    fn cells_sort_like_the_console() {
        assert_eq!(
            compare_cells(Some(&json!(2)), Some(&json!(10))),
            Ordering::Less
        );
        assert_eq!(
            compare_cells(Some(&json!("item 2")), Some(&json!("item 10"))),
            Ordering::Less
        );
        assert_eq!(
            compare_cells(Some(&json!("árbol")), Some(&json!("bote"))),
            Ordering::Less
        );
        assert_eq!(
            compare_cells(Some(&json!("b")), Some(&json!("B"))),
            Ordering::Less
        );
        assert_eq!(
            compare_cells(Some(&json!(null)), Some(&json!("a"))),
            Ordering::Greater
        );
        assert_eq!(compare_cells(Some(&json!("a")), None), Ordering::Less);
        assert_eq!(
            compare_cells(Some(&json!(5)), Some(&json!("10"))),
            Ordering::Less
        );
    }

    #[test]
    fn whitespace_collapses_in_cells() {
        assert_eq!(collapse("  a\n b\t c "), "a b c");
    }
}

#[cfg(test)]
mod card_band_tests {
    use super::card_band_token;

    #[test]
    fn states_use_their_edge_and_info_the_accent() {
        assert_eq!(card_band_token("success"), None);
        assert_eq!(card_band_token("danger"), None);
        assert_eq!(card_band_token("warning"), None);
        assert_eq!(card_band_token("info"), Some("accent"));
        assert_eq!(card_band_token("neutral"), Some("line-strong"));
    }
}
