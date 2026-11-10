//! "In progress" above a channel's stream (`WorkPanel`, `work-panel.tsx`).

use super::header::{eid, tip};
use crate::derive;
use crate::derive::work_items::{WORK_SHOWN, WorkItem};
use crate::i18n::{t, tv};
use crate::model::WorkspaceView;
use crate::robot;
use crate::theme::Theme;
use crate::ui::{self, SANS};
use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, App, FontWeight, InteractiveElement, IntoElement, ParentElement, RenderOnce,
    SharedString, StatefulInteractiveElement, Styled, Window, div, px, relative,
};
use std::rc::Rc;

/// Chooses a work item (`Some(key)`) or the whole channel (`None`).
pub type SelectWork = Rc<dyn Fn(Option<String>, &mut Window, &mut App)>;

/// The panel; `None` when there is no work item.
pub fn work_panel(
    view: &WorkspaceView,
    items: &[WorkItem],
    selected: Option<&str>,
    on_select: SelectWork,
    cx: &App,
) -> Option<AnyElement> {
    let _ = view;
    if items.is_empty() {
        return None;
    }
    let rows = items
        .iter()
        .map(|item| Row {
            key: item.key.clone(),
            title: item.title.clone().into(),
            line: item.line.clone().map(SharedString::from),
            status: item.status.label(cx),
            tone: item.status.tone(),
            faces: item
                .agents
                .iter()
                .take(3)
                .map(|a| {
                    robot::robot(derive::robot_name(a).to_string(), 22.)
                        .agent(a)
                        .dot(false)
                        .into_any_element()
                })
                .collect(),
        })
        .collect();
    Some(
        WorkPanel {
            rows,
            selected: selected.map(str::to_string),
            on_select,
        }
        .into_any_element(),
    )
}

/// What the head's button does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HeadAction {
    /// "See the whole channel": a row is chosen.
    WholeChannel,
    /// "Show all (n)", or "Show less" once open.
    SeeAll {
        open: bool,
    },
    None,
}

/// The rows shown (indexes into the items) and the head's button: the
/// chosen row alone, else all of them or the first [`WORK_SHOWN`].
pub fn shown_rows(count: usize, chosen: Option<usize>, all: bool) -> (Vec<usize>, HeadAction) {
    if let Some(i) = chosen.filter(|i| *i < count) {
        return (vec![i], HeadAction::WholeChannel);
    }
    if count > WORK_SHOWN {
        let n = if all { count } else { WORK_SHOWN };
        return ((0..n).collect(), HeadAction::SeeAll { open: all });
    }
    ((0..count).collect(), HeadAction::None)
}

/// The chosen row's index; a key no longer listed chooses nothing.
pub fn chosen<'a>(
    keys: impl IntoIterator<Item = &'a str>,
    selected: Option<&str>,
) -> Option<usize> {
    let selected = selected?;
    keys.into_iter().position(|k| k == selected)
}

struct Row {
    key: String,
    title: SharedString,
    line: Option<SharedString>,
    status: SharedString,
    tone: &'static str,
    faces: Vec<AnyElement>,
}

#[derive(IntoElement)]
struct WorkPanel {
    rows: Vec<Row>,
    selected: Option<String>,
    on_select: SelectWork,
}

impl RenderOnce for WorkPanel {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let all = window.use_keyed_state("hb-work-all", cx, |_, _| false);
        let open = *all.read(cx);
        let theme = cx.global::<Theme>();
        let count = self.rows.len();
        let chosen = chosen(
            self.rows.iter().map(|r| r.key.as_str()),
            self.selected.as_deref(),
        );
        let (shown, action) = shown_rows(count, chosen, open);
        let head_text = if chosen.is_some() {
            t(cx, "workspace.workOnlyThis")
        } else {
            tv(
                cx,
                "workspace.workInProgressCount",
                &[("n", &count.to_string())],
            )
        };
        let link = theme.c("link");
        let fold = |id: &'static str| {
            div()
                .id(id)
                .flex()
                .items_center()
                .whitespace_nowrap()
                .font_weight(FontWeight::BOLD)
                .text_color(link)
                .cursor_pointer()
                .hover(|s| s.underline())
        };
        let button = match action {
            HeadAction::WholeChannel => {
                let on_select = self.on_select.clone();
                Some(
                    fold("hb-work-whole")
                        .child(div().mr(px(4.)).child(ui::icon("x", 12., link)))
                        .child(t(cx, "workspace.workWholeChannel"))
                        .on_click(move |_, window, cx| on_select(None, window, cx)),
                )
            }
            HeadAction::SeeAll { open } => {
                let all = all.clone();
                Some(
                    fold("hb-work-all")
                        .child(if open {
                            t(cx, "workspace.workSeeLess")
                        } else {
                            tv(cx, "workspace.workSeeAll", &[("n", &count.to_string())])
                        })
                        .on_click(move |_, _, cx| {
                            all.update(cx, |all, cx| {
                                *all = !*all;
                                cx.notify();
                            })
                        }),
                )
            }
            HeadAction::None => None,
        };
        let row_title = t(cx, "workspace.workRowTitle");
        let selected = self.selected.clone();
        let mut rows: Vec<Option<Row>> = self.rows.into_iter().map(Some).collect();
        let rows: Vec<AnyElement> = shown
            .into_iter()
            .filter_map(|i| rows.get_mut(i).and_then(Option::take))
            .map(|row| {
                let pressed = selected.as_deref() == Some(row.key.as_str());
                let on_select = self.on_select.clone();
                let key = row.key.clone();
                let tone = match row.tone {
                    "green" => theme.c("green"),
                    "amber" => theme.c("amber"),
                    _ => theme.c("ink-3"),
                };
                div()
                    .id(eid("hb-work-row", &row.key))
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .w_full()
                    .min_w_0()
                    .px(px(20.))
                    .py(px(5.))
                    .text_color(theme.c("ink"))
                    .cursor_pointer()
                    .when(pressed, |d| d.bg(theme.c("hover")))
                    .hover(|s| s.bg(theme.c("hover")))
                    .tooltip(tip(row_title.clone()))
                    .child(div().flex_none().flex().gap(px(2.)).children(row.faces))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .flex_1()
                            .min_w_0()
                            .child(
                                div()
                                    .overflow_hidden()
                                    .whitespace_nowrap()
                                    .text_ellipsis()
                                    .text_size(px(14.))
                                    .font_weight(FontWeight::BOLD)
                                    .child(row.title),
                            )
                            .children(row.line.map(|line| {
                                div()
                                    .overflow_hidden()
                                    .whitespace_nowrap()
                                    .text_ellipsis()
                                    .text_size(px(13.))
                                    .text_color(theme.c("ink-3"))
                                    .child(line)
                            })),
                    )
                    .child(
                        div()
                            .flex_none()
                            .text_size(px(12.))
                            .font_weight(FontWeight::BOLD)
                            .text_color(tone)
                            .child(row.status),
                    )
                    .on_click(move |_, window, cx| {
                        on_select((!pressed).then(|| key.clone()), window, cx)
                    })
                    .into_any_element()
            })
            .collect();
        div()
            .id("hb-work")
            .flex_none()
            .flex()
            .flex_col()
            .max_h(relative(0.42))
            .overflow_y_scroll()
            .pt(px(8.))
            .pb(px(6.))
            .border_b_1()
            .border_color(theme.c("line"))
            .font_family(SANS)
            .child(
                div()
                    .flex()
                    .items_baseline()
                    .gap(px(10.))
                    .px(px(20.))
                    .pb(px(4.))
                    .text_size(px(13.))
                    .font_weight(FontWeight::BOLD)
                    .text_color(theme.c("ink-3"))
                    .child(head_text)
                    .children(button),
            )
            .children(rows)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn few_items_show_all_without_a_button() {
        assert_eq!(
            shown_rows(3, None, false),
            (vec![0, 1, 2], HeadAction::None)
        );
        assert_eq!(shown_rows(WORK_SHOWN, None, false).1, HeadAction::None);
    }

    #[test]
    fn many_items_fold_to_five_and_unfold() {
        let (rows, action) = shown_rows(8, None, false);
        assert_eq!(rows, vec![0, 1, 2, 3, 4]);
        assert_eq!(action, HeadAction::SeeAll { open: false });
        let (rows, action) = shown_rows(8, None, true);
        assert_eq!(rows.len(), 8);
        assert_eq!(action, HeadAction::SeeAll { open: true });
    }

    #[test]
    fn a_chosen_item_shows_alone_with_the_way_back() {
        assert_eq!(
            shown_rows(8, Some(6), false),
            (vec![6], HeadAction::WholeChannel)
        );
        assert_eq!(
            shown_rows(8, Some(6), true),
            (vec![6], HeadAction::WholeChannel)
        );
    }

    /// The sandbox carries no ticket keys: the panel's data path from the
    /// agents to the rows it shows.
    #[test]
    fn tickets_in_a_channel_become_rows_in_status_order() {
        use crate::derive::work_items::{WorkStatus, work_items};
        use crate::model::{Agent, Card, WorkspaceView};
        const NOW: u64 = 1_794_086_000_000;
        let agent = |key: &str, ticket: u32, state: &str| Agent {
            key: key.into(),
            name: key.into(),
            state: state.into(),
            channels: vec!["harbard".into()],
            last_action_ms: Some(NOW - u64::from(ticket % 10) * 1000),
            description: Some(format!("AB#{ticket} · Work {ticket}")),
            ..Default::default()
        };
        let mut agents: Vec<Agent> = (0..7)
            .map(|i| agent(&format!("a{i}"), 12340 + i, "running"))
            .collect();
        agents.push(agent("b0", 12340, "idle"));
        agents.push(Agent {
            branch: Some("feat/header-54321".into()),
            description: None,
            ..agent("c", 0, "idle")
        });
        let cards = vec![Card {
            kind: "question".into(),
            needs: true,
            agents: vec!["a6".into()],
            ..Default::default()
        }];
        let view = WorkspaceView {
            agents,
            cards,
            ..Default::default()
        };
        let items = work_items(&view, "harbard", &[], NOW);
        let keys: Vec<&str> = items.iter().map(|i| i.key.as_str()).collect();
        assert_eq!(keys.len(), 8);
        assert_eq!(keys[0], "12346");
        assert_eq!(items[0].status, WorkStatus::Needs);
        assert_eq!(
            items
                .iter()
                .find(|i| i.key == "12340")
                .map(|i| i.agents.len()),
            Some(2)
        );
        assert_eq!(
            items.last().map(|i| (i.key.as_str(), i.status)),
            Some(("54321", WorkStatus::Paused))
        );
        let (rows, action) = shown_rows(keys.len(), chosen(keys.iter().copied(), None), false);
        assert_eq!(rows.len(), WORK_SHOWN);
        assert_eq!(action, HeadAction::SeeAll { open: false });
        let pick = chosen(keys.iter().copied(), Some("54321"));
        assert_eq!(
            shown_rows(keys.len(), pick, false),
            (vec![7], HeadAction::WholeChannel)
        );
        assert_eq!(chosen(keys.iter().copied(), Some("99999")), None);
    }

    #[test]
    fn a_chosen_key_that_is_gone_shows_the_list() {
        assert_eq!(
            shown_rows(2, Some(5), false),
            (vec![0, 1], HeadAction::None)
        );
    }
}
