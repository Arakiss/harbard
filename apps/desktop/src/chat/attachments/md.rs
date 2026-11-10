//! `MarkdownView`: a report drawn from `derive::markdown` data (`.hb-md`).

use crate::derive::markdown::{Block, Inline};
use crate::theme::Theme;
use crate::ui;
use gpui::{
    AnyElement, App, Div, ElementId, FontStyle, FontWeight, InteractiveText, IntoElement,
    ParentElement, Styled, StyledText, TextRun, UnderlineStyle, div, font, px, relative,
};
use std::ops::Range;

/// One block of inline runs, flattened to a string and its styled runs.
#[derive(Default)]
struct Runs {
    text: String,
    runs: Vec<TextRun>,
    links: Vec<(Range<usize>, String)>,
}

#[derive(Clone, Copy)]
struct Style {
    weight: FontWeight,
    italic: bool,
    code: bool,
    /// Inside a link: Some(true) when it can be followed.
    link: Option<bool>,
}

fn push(runs: &mut Runs, s: &str, st: Style, theme: &Theme) {
    if s.is_empty() {
        return;
    }
    let mut f = font(if st.code { ui::MONO } else { ui::SANS });
    f.weight = st.weight;
    if st.italic {
        f.style = FontStyle::Italic;
    }
    let link = st.link == Some(true);
    runs.text.push_str(s);
    runs.runs.push(TextRun {
        len: s.len(),
        font: f,
        color: if link {
            theme.c("link")
        } else {
            theme.c("ink")
        },
        background_color: st.code.then(|| theme.c("control")),
        underline: link.then(|| UnderlineStyle {
            thickness: px(1.),
            color: Some(theme.c("link")),
            wavy: false,
        }),
        strikethrough: None,
    });
}

fn flatten(list: &[Inline], st: Style, out: &mut Runs, theme: &Theme) {
    for r in list {
        match r {
            Inline::Text(v) => push(out, v, st, theme),
            Inline::Code(v) => push(out, v, Style { code: true, ..st }, theme),
            Inline::Strong(c) => flatten(
                c,
                Style {
                    weight: if st.weight.0 > FontWeight::BOLD.0 {
                        st.weight
                    } else {
                        FontWeight::BOLD
                    },
                    ..st
                },
                out,
                theme,
            ),
            Inline::Em(c) => flatten(c, Style { italic: true, ..st }, out, theme),
            Inline::Link { href, c } => {
                let start = out.text.len();
                flatten(
                    c,
                    Style {
                        link: Some(href.is_some()),
                        ..st
                    },
                    out,
                    theme,
                );
                if let Some(href) = href {
                    out.links.push((start..out.text.len(), href.clone()));
                }
            }
        }
    }
}

/// Inline runs as one text element; links open in the browser.
fn runs(
    id: &str,
    list: &[Inline],
    weight: FontWeight,
    color_override: Option<gpui::Hsla>,
    theme: &Theme,
) -> AnyElement {
    let mut out = Runs::default();
    flatten(
        list,
        Style {
            weight,
            italic: false,
            code: false,
            link: None,
        },
        &mut out,
        theme,
    );
    if let Some(c) = color_override {
        for r in out.runs.iter_mut().filter(|r| r.underline.is_none()) {
            r.color = c;
        }
    }
    let styled = StyledText::new(out.text).with_runs(out.runs);
    if out.links.is_empty() {
        return styled.into_any_element();
    }
    let (ranges, hrefs): (Vec<_>, Vec<_>) = out.links.into_iter().unzip();
    InteractiveText::new(ElementId::Name(id.to_string().into()), styled)
        .on_click(ranges, move |i, _, cx| {
            if let Some(href) = hrefs.get(i) {
                cx.open_url(href);
            }
        })
        .into_any_element()
}

/// The report's blocks (`div.hb-md`).
pub(super) fn view(id: &str, blocks: &[Block], cx: &App) -> Div {
    let theme = cx.global::<Theme>();
    let mut root = div()
        .flex()
        .flex_col()
        .min_w_0()
        .font_family(ui::SANS)
        .text_size(px(15.))
        .line_height(relative(1.55))
        .text_color(theme.c("ink"));
    for (i, b) in blocks.iter().enumerate() {
        let bid = format!("{id}-{i}");
        let gap = if i == 0 { 0. } else { 10. };
        let el: AnyElement = match b {
            Block::Heading { level, c } => {
                // h1 reads as h2, and so on (`h{min(L+1,6)}`).
                let (size, top) = match (level + 1).min(6) {
                    2 => (22., gap),
                    3 => (18., 18.),
                    _ => (15., 14.),
                };
                div()
                    .mt(px(top))
                    .text_size(px(size))
                    .child(runs(&bid, c, FontWeight::BLACK, None, theme))
                    .into_any_element()
            }
            Block::Paragraph(c) => div()
                .mt(px(gap))
                .child(runs(&bid, c, FontWeight::NORMAL, None, theme))
                .into_any_element(),
            Block::Quote(c) => div()
                .mt(px(gap))
                .pl(px(12.))
                .border_l(px(3.))
                .border_color(theme.c("line-strong"))
                .child(runs(
                    &bid,
                    c,
                    FontWeight::NORMAL,
                    Some(theme.c("ink-2")),
                    theme,
                ))
                .into_any_element(),
            Block::List { ordered, items } => {
                let rows: Vec<AnyElement> = items
                    .iter()
                    .enumerate()
                    .map(|(n, item)| {
                        let marker = if *ordered {
                            format!("{}.", n + 1)
                        } else {
                            "•".to_string()
                        };
                        div()
                            .flex()
                            .child(
                                div()
                                    .w(px(22.))
                                    .flex_none()
                                    .flex()
                                    .justify_end()
                                    .pr(px(6.))
                                    .child(marker),
                            )
                            .child(div().flex_1().min_w_0().child(runs(
                                &format!("{bid}-{n}"),
                                item,
                                FontWeight::NORMAL,
                                None,
                                theme,
                            )))
                            .into_any_element()
                    })
                    .collect();
                div()
                    .mt(px(gap))
                    .flex()
                    .flex_col()
                    .children(rows)
                    .into_any_element()
            }
            Block::Code { v, .. } => div()
                .mt(px(gap))
                .px(px(12.))
                .py(px(10.))
                .rounded(px(6.))
                .border_1()
                .border_color(theme.c("line"))
                .bg(theme.c("surface-2"))
                .font_family(ui::MONO)
                .text_size(px(13.))
                .child(v.clone())
                .into_any_element(),
            Block::Rule => div()
                .mt(px(gap))
                .h(px(1.))
                .bg(theme.c("line"))
                .into_any_element(),
            Block::Table { head, rows } => table(&bid, head, rows, gap, theme),
        };
        root = root.child(el);
    }
    root
}

/// A pipe table: equal columns, collapsed borders, a shaded head row.
fn table(
    id: &str,
    head: &[Vec<Inline>],
    rows: &[Vec<Vec<Inline>>],
    gap: f32,
    theme: &Theme,
) -> AnyElement {
    let cols = rows
        .iter()
        .map(Vec::len)
        .chain([head.len()])
        .max()
        .unwrap_or(1)
        .max(1);
    let cell = |key: String, c: &[Inline], th: bool| {
        let mut d = div()
            .w(relative(1. / cols as f32))
            .min_w_0()
            .px(px(10.))
            .py(px(4.))
            .border_r_1()
            .border_b_1()
            .border_color(theme.c("line"));
        if th {
            d = d.bg(theme.c("surface-2"));
        }
        let weight = if th {
            FontWeight::BLACK
        } else {
            FontWeight::NORMAL
        };
        d.child(runs(&key, c, weight, None, theme))
    };
    let row = |r: usize, cells: &[Vec<Inline>], th: bool| {
        div().flex().children(
            cells
                .iter()
                .enumerate()
                .map(|(i, c)| cell(format!("{id}-{r}-{i}"), c, th)),
        )
    };
    div()
        .mt(px(gap))
        .flex()
        .flex_col()
        .text_size(px(14.))
        .border_t_1()
        .border_l_1()
        .border_color(theme.c("line"))
        .child(row(0, head, true))
        .children(
            rows.iter()
                .enumerate()
                .map(|(r, cells)| row(r + 1, cells, false)),
        )
        .into_any_element()
}
