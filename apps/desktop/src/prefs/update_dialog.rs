//! "Update harbard" (`update-dialog.tsx`): the installed version beside the
//! newer one, what it brings (the notes, then every commit grouped the way a
//! changelog groups them) and one button. Once a run starts, the same dialog
//! shows its steps live, its log, and how it ended.

use super::shell_update::{APP_COMMIT, UpdateStore, changes_count, page_is_stale};
use super::stores::{check_updates, dismiss_update, install_update, update_running};
use super::updates_page::warn;
use super::widgets::{self, Kind, Size};
use crate::derive::update::{
    DetailAt, brought_by, channel_label, group_changes, run_steps, sentence_case, short_sha,
    source_problem_text, status_chip, status_sentence, step_detail, step_elapsed, step_label,
    target_name, target_when, update_error_text,
};
use crate::i18n::t;
use crate::model::update::{UpdateChange, UpdateRun};
use crate::theme::Theme;
use crate::ui;
use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, App, AppContext, Context, FontWeight, InteractiveElement, IntoElement,
    ParentElement, Render, ScrollHandle, SharedString, StatefulInteractiveElement, Styled, Task,
    Window, div, point, px,
};
use std::time::Duration;

pub struct UpdateDialog {
    log_open: bool,
    /// `None` = the default (open when there are no notes).
    commits_open: Option<bool>,
    log_scroll: ScrollHandle,
    _clock: Task<()>,
}

impl UpdateDialog {
    fn new(cx: &mut Context<Self>) -> Self {
        cx.observe(&UpdateStore::get(cx), |this: &mut Self, _, cx| {
            if this.log_open {
                this.log_scroll.set_offset(point(px(0.), px(-1.0e6)));
            }
            cx.notify();
        })
        .detach();
        cx.observe_global::<Theme>(|_, cx| cx.notify()).detach();
        // Step clocks tick each second while a run is live.
        let clock = cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(Duration::from_secs(1)).await;
                let Ok(live) =
                    this.update(cx, |_, cx| update_running(UpdateStore::get(cx).read(cx)))
                else {
                    break;
                };
                if live && this.update(cx, |_, cx| cx.notify()).is_err() {
                    break;
                }
            }
        });
        UpdateDialog {
            log_open: false,
            commits_open: None,
            log_scroll: ScrollHandle::new(),
            _clock: clock,
        }
    }
}

pub fn open(window: &mut Window, cx: &mut App) {
    let view = cx.new(UpdateDialog::new);
    widgets::open_modal(
        view.into(),
        680.,
        None,
        widgets::Place::Top(0.08),
        None,
        window,
        cx,
    );
}

/// What the dialog reads from the update store, copied for one render.
struct Snap {
    view: Option<crate::model::update::UpdateView>,
    checking: bool,
    installing: bool,
    restarting: bool,
    followed: Option<String>,
    error: Option<SharedString>,
}

impl Snap {
    fn of(s: &UpdateStore) -> Snap {
        Snap {
            view: s.view.clone(),
            checking: s.checking,
            installing: s.installing,
            restarting: s.restarting,
            followed: s.followed.clone(),
            error: s.error.clone(),
        }
    }

    /// Progress, rather than the changelog, while a run goes on or this app followed one.
    fn shows_run(&self) -> bool {
        if self.installing || self.restarting {
            return true;
        }
        let Some(run) = self.view.as_ref().and_then(|v| v.run.as_ref()) else {
            return false;
        };
        run.state == "running"
            || self.followed.as_deref() == Some(run.id.as_str())
            || (run.state == "done" && page_is_stale(run, APP_COMMIT))
    }
}

/// The pane head: title and X (`.hb-pane-head`).
pub fn pane_head(title: impl Into<SharedString>, theme: &Theme) -> gpui::Div {
    div()
        .flex()
        .flex_none()
        .items_center()
        .gap(px(8.))
        .h(px(56.))
        .pl(px(18.))
        .pr(px(12.))
        .border_b_1()
        .border_color(theme.c("line"))
        .child(
            div()
                .text_size(px(18.))
                .font_weight(FontWeight::BLACK)
                .child(title.into()),
        )
        .child(widgets::close_button("pane-close", theme, |_, w, cx| {
            widgets::close_top(w, cx)
        }))
}

/// A markdown note, read plainly: headings, bullets and paragraphs.
fn notes_view(source: &str, theme: &Theme) -> AnyElement {
    let mut out = div().flex().flex_col().gap(px(6.)).text_size(px(15.));
    let mut para = String::new();
    let flush = |out: gpui::Div, para: &mut String| {
        if para.trim().is_empty() {
            para.clear();
            return out;
        }
        let text = std::mem::take(para);
        out.child(div().child(text.trim().replace("**", "").replace('`', "")))
    };
    for line in source.lines() {
        let l = line.trim_end();
        if let Some(h) = l.trim_start().strip_prefix('#') {
            out = flush(out, &mut para);
            out = out.child(
                div()
                    .mt(px(4.))
                    .text_size(px(16.))
                    .font_weight(FontWeight::BLACK)
                    .child(h.trim_start_matches('#').trim().replace("**", "")),
            );
        } else if let Some(item) = l
            .trim_start()
            .strip_prefix("- ")
            .or_else(|| l.trim_start().strip_prefix("* "))
        {
            out = flush(out, &mut para);
            out = out.child(
                div()
                    .flex()
                    .gap(px(8.))
                    .pl(px(4.))
                    .child(div().text_color(theme.c("ink-3")).child("•"))
                    .child(
                        div()
                            .flex_1()
                            .child(item.replace("**", "").replace('`', "")),
                    ),
            );
        } else if l.trim().is_empty() {
            out = flush(out, &mut para);
        } else {
            if !para.is_empty() {
                para.push(' ');
            }
            para.push_str(l.trim());
        }
    }
    flush(out, &mut para).into_any_element()
}

impl UpdateDialog {
    fn render_offer(
        &self,
        store: &Snap,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> (AnyElement, AnyElement) {
        let now = harbard_core::clock::now_ms();
        let view = store.view.clone();
        let check = view.as_ref().and_then(|v| v.check.clone());
        let available = check.as_ref().is_some_and(|c| c.status == "available");
        let problem = view.as_ref().and_then(|v| v.source.problem.clone());
        let run = view.as_ref().and_then(|v| v.run.clone());
        let brought = if available {
            None
        } else {
            brought_by(run.as_ref())
        };
        let notes: String = match &brought {
            Some(b) => b.notes.unwrap_or_default().to_string(),
            None => check
                .as_ref()
                .and_then(|c| c.notes.clone())
                .unwrap_or_default()
                .trim()
                .to_string(),
        };
        let changes: Vec<UpdateChange> = match &brought {
            Some(b) => b.changes.to_vec(),
            None if available => check
                .as_ref()
                .map(|c| c.changes.clone())
                .unwrap_or_default(),
            None => Vec::new(),
        };
        let truncated = match &brought {
            Some(b) => b.truncated,
            None => check.as_ref().is_some_and(|c| c.changes_truncated),
        };
        let (tone, chip_text) = if store.checking {
            ("", t(cx, "update.checking"))
        } else {
            status_chip(check.as_ref(), cx)
        };
        let meta: SharedString = if store.checking {
            t(cx, "update.checkingRepository")
        } else if let (true, Some(c)) = (available, &check) {
            let mut parts = vec![changes_count(c.behind, cx).to_string()];
            if let Some(when) = target_when(c, now, cx) {
                parts
                    .push(crate::i18n::tv(cx, "update.metaLatest", &[("when", &when)]).to_string());
            }
            let channel = channel_label(
                if c.channel.is_empty() {
                    view.as_ref().map_or("main", |v| v.prefs.channel.as_str())
                } else {
                    &c.channel
                },
                cx,
            );
            parts.push(
                crate::i18n::tv(
                    cx,
                    "update.metaChannel",
                    &[("channel", &channel.to_lowercase())],
                )
                .to_string(),
            );
            parts.join(" · ").into()
        } else {
            status_sentence(view.as_ref(), now, cx)
        };

        let version = |label: SharedString, version: String, sha: String, new: bool| {
            div()
                .flex()
                .flex_col()
                .line_height(px(25.))
                .child(
                    div()
                        .text_size(px(12.))
                        .line_height(px(15.))
                        .font_weight(FontWeight::BOLD)
                        .text_color(theme.c("ink-3"))
                        .child(label.to_uppercase()),
                )
                .child(
                    div()
                        .text_size(px(20.))
                        .font_weight(FontWeight::BLACK)
                        .when(new, |d| d.text_color(theme.c("accent-text")))
                        .child(version),
                )
                .when(!sha.is_empty(), |d| {
                    d.child(
                        div()
                            .font_family(ui::MONO)
                            .text_size(px(13.))
                            .line_height(px(16.))
                            .text_color(theme.c("ink-3"))
                            .child(sha),
                    )
                })
        };

        let hero = div()
            .flex()
            .flex_col()
            .items_start()
            .gap(px(10.))
            .px(px(18.))
            .py(px(16.))
            .rounded(px(10.))
            .bg(theme.c("tint"))
            .child(widgets::chip(chip_text, tone, theme))
            .when_some(view.clone(), |d, v| {
                let installed = version(
                    t(cx, "update.versionYours"),
                    v.installed.version.clone(),
                    short_sha(v.installed.commit.as_deref()),
                    false,
                );
                let target = check.as_ref().filter(|_| available).and_then(|c| {
                    c.target.as_ref().map(|tg| {
                        version(
                            t(cx, "update.versionNew"),
                            target_name(c, &v.installed.version),
                            short_sha(Some(&tg.commit)),
                            true,
                        )
                    })
                });
                d.child(
                    div()
                        .flex()
                        .flex_wrap()
                        .items_center()
                        .gap(px(14.))
                        .child(installed)
                        .when_some(target, |d, tg| {
                            d.child(ui::icon("arrow-right", 20., theme.c("ink-4")))
                                .child(tg)
                        }),
                )
            })
            .child(
                div()
                    .text_size(px(14.))
                    .text_color(theme.c("ink-2"))
                    .child(meta),
            )
            .when_some(
                check
                    .as_ref()
                    .and_then(|c| c.target.as_ref())
                    .and_then(|tg| tg.title.clone()),
                |d, title| d.child(div().font_weight(FontWeight::BOLD).child(title)),
            );

        let groups = group_changes(&changes);
        let commits_open = self.commits_open.unwrap_or(notes.is_empty());
        let count = if available {
            check.as_ref().map_or(changes.len() as u64, |c| c.behind)
        } else {
            changes.len() as u64
        };

        let body = div()
            .flex()
            .flex_col()
            .gap(px(16.))
            .child(hero)
            .when_some(problem.clone(), |d, p| {
                let path = view
                    .as_ref()
                    .and_then(|v| v.source.path.clone())
                    .unwrap_or_default();
                d.child(warn(
                    format!("{} {}", source_problem_text(&p, cx), path)
                        .trim()
                        .to_string(),
                    false,
                    theme,
                ))
            })
            .when_some(store.error.clone(), |d, e| d.child(warn(e, true, theme)))
            .map(|d| {
                if !notes.is_empty() {
                    let title = if available {
                        t(cx, "update.notesTitle")
                    } else if brought.is_some() {
                        t(cx, "update.notesBrought")
                    } else {
                        t(cx, "update.notesCurrent")
                    };
                    d.child(
                        div()
                            .child(
                                div()
                                    .mb(px(8.))
                                    .text_size(px(17.))
                                    .font_weight(FontWeight::BLACK)
                                    .child(title),
                            )
                            .child(notes_view(&notes, theme)),
                    )
                } else if available {
                    d.child(widgets::hint(t(cx, "update.noNotes"), theme))
                } else {
                    d
                }
            })
            .when(!groups.is_empty(), |d| {
                d.child(
                    div()
                        .pt(px(10.))
                        .border_t_1()
                        .border_color(theme.c("line"))
                        .child(
                            div()
                                .id("commits-toggle")
                                .flex()
                                .items_center()
                                .gap(px(6.))
                                .cursor_pointer()
                                .font_weight(FontWeight::BLACK)
                                .child(ui::icon(
                                    if commits_open {
                                        "chevron-down"
                                    } else {
                                        "chevron-right"
                                    },
                                    16.,
                                    theme.c("ink-3"),
                                ))
                                .child(t(cx, "update.commitsTitle"))
                                .child(
                                    div()
                                        .px(px(7.))
                                        .rounded(px(9.))
                                        .text_size(px(12.))
                                        .bg(theme.c("chip"))
                                        .text_color(theme.c("chip-ink"))
                                        .child(count.to_string()),
                                )
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.commits_open = Some(!commits_open);
                                    cx.notify();
                                })),
                        )
                        .when(commits_open, |d| {
                            d.children(groups.iter().map(|(g, list)| {
                                let n = list.len();
                                div()
                                    .mt(px(12.))
                                    .child(
                                        div()
                                            .mb(px(4.))
                                            .text_size(px(13.))
                                            .font_weight(FontWeight::BLACK)
                                            .text_color(theme.c("ink-3"))
                                            .child(g.label(cx).to_uppercase()),
                                    )
                                    .children(list.iter().enumerate().map(|(i, c)| {
                                        div()
                                            .flex()
                                            .items_baseline()
                                            .gap(px(8.))
                                            .py(px(4.))
                                            .when(i + 1 < n, |d| {
                                                d.border_b_1().border_color(theme.c("line"))
                                            })
                                            .child(
                                                div()
                                                    .flex_none()
                                                    .font_family(ui::MONO)
                                                    .text_size(px(13.))
                                                    .text_color(theme.c("ink-3"))
                                                    .child(short_sha(Some(&c.commit))),
                                            )
                                            .when_some(c.scope.clone(), |d, s| {
                                                d.child(
                                                    div()
                                                        .flex_none()
                                                        .px(px(6.))
                                                        .rounded(px(5.))
                                                        .text_size(px(12.))
                                                        .font_weight(FontWeight::BOLD)
                                                        .bg(theme.c("chip"))
                                                        .text_color(theme.c("chip-ink"))
                                                        .child(s),
                                                )
                                            })
                                            .child(div().min_w_0().child(sentence_case(&c.subject)))
                                    }))
                            }))
                            .when(truncated, |d| {
                                d.child(
                                    div()
                                        .mt(px(8.))
                                        .child(widgets::hint(t(cx, "update.truncatedHint"), theme)),
                                )
                            })
                        }),
                )
            })
            .into_any_element();

        let foot = if available {
            div()
                .flex()
                .gap(px(10.))
                .child(widgets::button(
                    "update-later",
                    t(cx, "update.later"),
                    Kind::Plain,
                    Size::Normal,
                    None,
                    false,
                    theme,
                    |_, w, cx| {
                        dismiss_update(cx);
                        widgets::close_top(w, cx);
                    },
                ))
                .child(widgets::button(
                    "update-now",
                    t(cx, "update.updateNow"),
                    Kind::Primary,
                    Size::Normal,
                    None,
                    problem.is_some() || store.installing,
                    theme,
                    |_, _, cx| install_update(cx),
                ))
        } else {
            div()
                .flex()
                .gap(px(10.))
                .child(widgets::button(
                    "update-check",
                    if store.checking {
                        t(cx, "update.checking")
                    } else {
                        t(cx, "update.checkNow")
                    },
                    Kind::Plain,
                    Size::Normal,
                    store.checking.then_some("loader-circle"),
                    store.checking || view.is_none(),
                    theme,
                    |_, _, cx| check_updates(cx),
                ))
                .child(widgets::button(
                    "update-close",
                    t(cx, "update.close"),
                    Kind::Primary,
                    Size::Normal,
                    None,
                    false,
                    theme,
                    |_, w, cx| widgets::close_top(w, cx),
                ))
        };
        (body, foot.into_any_element())
    }

    fn render_run(
        &self,
        store: &Snap,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> (AnyElement, AnyElement) {
        let now = harbard_core::clock::now_ms();
        let run: Option<UpdateRun> = store.view.as_ref().and_then(|v| v.run.clone());
        let failed = run.as_ref().is_some_and(|r| r.state == "failed");
        let done = run.as_ref().is_some_and(|r| r.state == "done") && !store.restarting;
        let stale = run.as_ref().is_some_and(|r| page_is_stale(r, APP_COMMIT));
        let (title, hint) = if store.restarting {
            (t(cx, "update.restarting"), t(cx, "update.restartingHint"))
        } else if failed {
            (
                t(cx, "update.failedTitle"),
                update_error_text(run.as_ref().and_then(|r| r.error.as_ref()), cx),
            )
        } else if done {
            (
                t(cx, "update.done"),
                if stale {
                    t(cx, "update.doneHint")
                } else {
                    t(cx, "update.doneCurrent")
                },
            )
        } else {
            (t(cx, "update.bannerRunning"), t(cx, "update.runningHint"))
        };
        let (badge_bg, badge_ink, badge_icon) = if done {
            (theme.c("green-bg"), theme.c("green-ink"), "check")
        } else if failed {
            (theme.c("red-bg"), theme.c("red-ink"), "x")
        } else {
            (
                theme.c("accent-soft"),
                theme.c("accent-text"),
                "loader-circle",
            )
        };
        let steps = run_steps(run.as_ref());
        let log: Vec<String> = run.as_ref().map(|r| r.log_tail.clone()).unwrap_or_default();
        let body = div()
            .flex()
            .flex_col()
            .gap(px(16.))
            .child(
                div()
                    .flex()
                    .items_start()
                    .gap(px(14.))
                    .child(
                        div()
                            .flex_none()
                            .size(px(36.))
                            .rounded_full()
                            .flex()
                            .items_center()
                            .justify_center()
                            .bg(badge_bg)
                            .child(ui::icon(badge_icon, 20., badge_ink)),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .child(
                                div()
                                    .text_size(px(18.))
                                    .font_weight(FontWeight::BLACK)
                                    .child(title),
                            )
                            .child(
                                div()
                                    .text_size(px(14.))
                                    .text_color(theme.c("ink-3"))
                                    .child(hint),
                            ),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .rounded(px(10.))
                    .border_1()
                    .border_color(theme.c("line"))
                    .overflow_hidden()
                    .children(steps.iter().enumerate().map(|(i, s)| {
                        let (icon, ink, bg) = match s.state.as_str() {
                            "running" => ("loader-circle", theme.c("accent-text"), None),
                            "done" => ("check", theme.c("green-ink"), Some(theme.c("green-bg"))),
                            "failed" => ("x", theme.c("red-ink"), Some(theme.c("red-bg"))),
                            "skipped" => ("minus", theme.c("ink-4"), None),
                            _ => ("circle", theme.c("ink-4"), None),
                        };
                        let quiet = s.state == "pending" || s.state == "skipped";
                        let detail = step_detail(s, cx);
                        let side = match &detail {
                            Some((text, DetailAt::Side)) => Some(text.to_string()),
                            _ => step_elapsed(s, now),
                        };
                        div()
                            .flex()
                            .items_center()
                            .gap(px(12.))
                            .px(px(14.))
                            .py(px(10.))
                            .when(i > 0, |d| d.border_t_1().border_color(theme.c("line")))
                            .when(s.state == "running", |d| d.bg(theme.c("tint")))
                            .child(
                                div()
                                    .flex_none()
                                    .size(px(24.))
                                    .rounded_full()
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .when_some(bg, |d, bg| d.bg(bg))
                                    .child(ui::icon(icon, 16., ink)),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .flex()
                                    .flex_col()
                                    .child(
                                        div()
                                            .font_weight(FontWeight::BOLD)
                                            .when(quiet, |d| d.text_color(theme.c("ink-3")))
                                            .child(step_label(&s.id, cx)),
                                    )
                                    .when_some(
                                        detail.filter(|d| d.1 == DetailAt::Below),
                                        |d, (text, _)| {
                                            d.child(
                                                div()
                                                    .font_family(ui::MONO)
                                                    .text_size(px(13.))
                                                    .text_color(theme.c("ink-3"))
                                                    .child(text),
                                            )
                                        },
                                    ),
                            )
                            .when_some(side, |d, side| {
                                d.child(
                                    div()
                                        .flex_none()
                                        .text_size(px(13.))
                                        .text_color(theme.c("ink-3"))
                                        .child(side),
                                )
                            })
                    })),
            )
            .when(!log.is_empty(), |d| {
                let open = self.log_open;
                d.child(
                    div()
                        .child(
                            div()
                                .id("update-log-toggle")
                                .flex()
                                .items_center()
                                .gap(px(6.))
                                .cursor_pointer()
                                .font_weight(FontWeight::BOLD)
                                .text_size(px(14.))
                                .text_color(theme.c("ink-2"))
                                .child(ui::icon(
                                    if open {
                                        "chevron-down"
                                    } else {
                                        "chevron-right"
                                    },
                                    16.,
                                    theme.c("ink-3"),
                                ))
                                .child(if open {
                                    t(cx, "update.hideLog")
                                } else {
                                    t(cx, "update.showLog")
                                })
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.log_open = !this.log_open;
                                    this.log_scroll.set_offset(point(px(0.), px(-1.0e6)));
                                    cx.notify();
                                })),
                        )
                        .when(open, |d| {
                            d.child(
                                div()
                                    .id("update-log")
                                    .mt(px(8.))
                                    .max_h(px(220.))
                                    .overflow_y_scroll()
                                    .track_scroll(&self.log_scroll)
                                    .px(px(12.))
                                    .py(px(10.))
                                    .rounded(px(8.))
                                    .bg(theme.c("st-term"))
                                    .text_color(theme.c("st-term-ink"))
                                    .font_family(ui::MONO)
                                    .text_size(px(12.))
                                    .line_height(px(18.))
                                    .children(log.iter().map(|l| div().child(l.clone()))),
                            )
                        }),
                )
            })
            .into_any_element();

        let close = |id: &'static str, kind: Kind| {
            widgets::button(
                id,
                t(cx, "update.close"),
                kind,
                Size::Normal,
                None,
                false,
                theme,
                |_, w, cx| widgets::close_top(w, cx),
            )
        };
        let foot = if failed {
            div()
                .flex()
                .gap(px(10.))
                .child(close("run-close", Kind::Plain))
                .child(widgets::button(
                    "run-retry",
                    t(cx, "update.retry"),
                    Kind::Primary,
                    Size::Normal,
                    None,
                    false,
                    theme,
                    |_, _, cx| install_update(cx),
                ))
        } else if done {
            // The app restarts itself after an update; "Reload" has nothing to reload here.
            div().child(close("run-close", Kind::Plain))
        } else {
            div().child(close("run-close", Kind::Plain))
        };
        (body, foot.into_any_element())
    }
}

impl Render for UpdateDialog {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.global::<Theme>().clone();
        let snap = Snap::of(UpdateStore::get(cx).read(cx));
        let max_h = f32::from(window.viewport_size().height) * 0.84;
        let (body, foot) = if snap.shows_run() {
            self.render_run(&snap, &theme, cx)
        } else {
            self.render_offer(&snap, &theme, cx)
        };
        widgets::base(&theme)
            .flex()
            .flex_col()
            .max_h(px(max_h))
            .child(pane_head(t(cx, "update.dialogTitle"), &theme))
            .child(
                div()
                    .id("update-body")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .px(px(22.))
                    .pt(px(18.))
                    .pb(px(8.))
                    .child(body),
            )
            .child(
                div()
                    .flex()
                    .flex_none()
                    .justify_end()
                    .gap(px(10.))
                    .px(px(22.))
                    .py(px(14.))
                    .border_t_1()
                    .border_color(theme.c("line"))
                    .child(foot),
            )
    }
}
