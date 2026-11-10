//! The Preferences dialog (`preferences.tsx`): a 920 × 720 modal with a
//! left nav of five pages, each opening with its icon, title and summary.
//!
//! ```text
//! ┌ Preferences ─────────────────────────────── [x] ┐  head 64
//! │ nav 200 │ page head (icon 44, title, hint)       │
//! │ ▣ Appear│ section · section · …   (scrolls)      │
//! │ ▣ Genera│                                        │
//! ├─────────┴──────────────────────────────── [Done]┤  foot
//! ```

use super::appearance::AppearancePage;
use super::general::{GeneralPage, NotificationsPage};
use super::pairing::PairingPage;
use super::updates_page::UpdatesPage;
use super::widgets::{self, Kind, Size};
use crate::i18n::t;
use crate::theme::Theme;
use crate::ui;
use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyView, App, AppContext, Context, Entity, FontWeight, Global, InteractiveElement, IntoElement,
    ParentElement, Render, ScrollHandle, SharedString, StatefulInteractiveElement, Styled,
    WeakEntity, Window, div, point, px,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Page {
    Appearance,
    General,
    Notifications,
    Teams,
    Updates,
}

impl Page {
    pub const ALL: [Page; 5] = [
        Page::Appearance,
        Page::General,
        Page::Notifications,
        Page::Teams,
        Page::Updates,
    ];

    pub fn parse(s: &str) -> Option<Page> {
        Some(match s {
            "appearance" | "themes" => Page::Appearance,
            "general" | "language" => Page::General,
            "notifications" => Page::Notifications,
            "teams" | "pairing" => Page::Teams,
            "updates" | "update" => Page::Updates,
            _ => return None,
        })
    }

    fn icon(self) -> &'static str {
        match self {
            Page::Appearance => "palette",
            Page::General => "sliders-horizontal",
            Page::Notifications => "bell",
            Page::Teams => "users",
            Page::Updates => "refresh-cw",
        }
    }

    fn title(self, cx: &App) -> SharedString {
        t(
            cx,
            match self {
                Page::Appearance => "preferences.pageAppearance",
                Page::General => "preferences.pageGeneral",
                Page::Notifications => "preferences.notifications",
                Page::Teams => "preferences.teams",
                Page::Updates => "preferences.updates",
            },
        )
    }

    fn hint(self, cx: &App) -> SharedString {
        t(
            cx,
            match self {
                Page::Appearance => "preferences.pageAppearanceHint",
                Page::General => "preferences.pageGeneralHint",
                Page::Notifications => "preferences.pageNotificationsHint",
                Page::Teams => "preferences.pageTeamsHint",
                Page::Updates => "preferences.pageUpdatesHint",
            },
        )
    }
}

/// The open dialog, so a second "Preferences" moves it instead of stacking one.
#[derive(Default)]
struct OpenPrefs(Option<WeakEntity<PreferencesDialog>>);

impl Global for OpenPrefs {}

pub struct PreferencesDialog {
    page: Page,
    scroll: ScrollHandle,
    appearance: Entity<AppearancePage>,
    /// The other pages are built when first shown; Teams is dropped when
    /// left so its 15 s poll stops.
    general: Option<Entity<GeneralPage>>,
    notifications: Option<Entity<NotificationsPage>>,
    teams: Option<Entity<PairingPage>>,
    updates: Option<Entity<UpdatesPage>>,
}

impl PreferencesDialog {
    fn new(page: Page, window: &mut Window, cx: &mut Context<Self>) -> Self {
        cx.observe_global::<Theme>(|_, cx| cx.notify()).detach();
        cx.observe_global::<crate::i18n::I18n>(|_, cx| cx.notify())
            .detach();
        let appearance = cx.new(|cx| AppearancePage::new(window, cx));
        let mut this = PreferencesDialog {
            page: Page::Appearance,
            scroll: ScrollHandle::new(),
            appearance,
            general: None,
            notifications: None,
            teams: None,
            updates: None,
        };
        this.show(page, window, cx);
        this
    }

    pub fn show(&mut self, page: Page, window: &mut Window, cx: &mut Context<Self>) {
        self.page = page;
        self.scroll.set_offset(point(px(0.), px(0.)));
        match page {
            Page::General if self.general.is_none() => {
                self.general = Some(cx.new(|cx| GeneralPage::new(window, cx)))
            }
            Page::Notifications if self.notifications.is_none() => {
                self.notifications = Some(cx.new(|cx| NotificationsPage::new(window, cx)))
            }
            Page::Teams if self.teams.is_none() => {
                self.teams = Some(cx.new(|cx| PairingPage::new(window, cx)))
            }
            Page::Updates if self.updates.is_none() => {
                self.updates = Some(cx.new(|cx| UpdatesPage::new(window, cx)))
            }
            _ => {}
        }
        if page != Page::Teams {
            self.teams = None;
        }
        cx.notify();
    }

    fn current(&self) -> Option<AnyView> {
        match self.page {
            Page::Appearance => Some(self.appearance.clone().into()),
            Page::General => self.general.clone().map(Into::into),
            Page::Notifications => self.notifications.clone().map(Into::into),
            Page::Teams => self.teams.clone().map(Into::into),
            Page::Updates => self.updates.clone().map(Into::into),
        }
    }
}

/// Closing restores the chosen theme (a hover preview or an unsaved custom
/// one never outlives the dialog) and forgets the open dialog.
fn on_closed(_window: &mut Window, cx: &mut App) {
    cx.set_global(OpenPrefs(None));
    super::appearance::end_preview(cx);
}

pub fn open(section: Option<&str>, window: &mut Window, cx: &mut App) {
    let page = section.and_then(Page::parse).unwrap_or(Page::Appearance);
    if let Some(open) = cx
        .try_global::<OpenPrefs>()
        .and_then(|o| o.0.as_ref())
        .and_then(|w| w.upgrade())
    {
        open.update(cx, |d, cx| d.show(page, window, cx));
        return;
    }
    let dialog = cx.new(|cx| PreferencesDialog::new(page, window, cx));
    cx.set_global(OpenPrefs(Some(dialog.downgrade())));
    widgets::open_modal(
        dialog.into(),
        920.,
        Some(720.),
        widgets::Place::Center,
        Some(std::rc::Rc::new(on_closed)),
        window,
        cx,
    );
}

fn close(window: &mut Window, cx: &mut App) {
    on_closed(window, cx);
    widgets::close_top(window, cx);
}

impl Render for PreferencesDialog {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.global::<Theme>().clone();
        let page = self.page;
        let head = div()
            .flex()
            .flex_none()
            .items_center()
            .h(px(64.))
            .pl(px(28.))
            .pr(px(16.))
            .border_b_1()
            .border_color(theme.c("line"))
            .child(
                div()
                    .text_size(px(22.))
                    .font_weight(FontWeight::BLACK)
                    .child(t(cx, "preferences.preferences")),
            )
            .child(widgets::close_button("prefs-close", &theme, |_, w, cx| {
                close(w, cx)
            }));

        let nav = div()
            .w(px(200.))
            .flex_none()
            .px(px(10.))
            .py(px(16.))
            .border_r_1()
            .border_color(theme.c("line"))
            .children(Page::ALL.into_iter().map(|p| {
                let on = p == page;
                let hover = theme.c("hover-2");
                div()
                    .id(SharedString::from(format!("prefs-nav-{p:?}")))
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .mb(px(2.))
                    .px(px(10.))
                    .py(px(6.))
                    .rounded(px(8.))
                    .font_weight(FontWeight::BOLD)
                    .cursor_pointer()
                    .when(on, |d| {
                        d.bg(theme.c("accent")).text_color(theme.c("accent-ink"))
                    })
                    .when(!on, |d| d.hover(move |s| s.bg(hover)))
                    .child(
                        div()
                            .flex_none()
                            .size(px(26.))
                            .rounded(px(7.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .bg(if on {
                                theme.c("accent-deep")
                            } else {
                                theme.c("accent-soft")
                            })
                            .child(ui::icon(
                                p.icon(),
                                16.,
                                if on {
                                    theme.c("accent-ink")
                                } else {
                                    theme.c("accent-text")
                                },
                            )),
                    )
                    .child(p.title(cx))
                    .on_click(cx.listener(move |this, _, window, cx| this.show(p, window, cx)))
            }));

        let page_head = div()
            .flex()
            .items_center()
            .gap(px(14.))
            .pt(px(18.))
            .pb(px(16.))
            .border_b_1()
            .border_color(theme.c("line"))
            .child(
                div()
                    .flex_none()
                    .size(px(44.))
                    .rounded(px(11.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .bg(theme.c("accent"))
                    .child(ui::icon(page.icon(), 24., theme.c("accent-ink"))),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .text_size(px(21.))
                            .line_height(px(25.))
                            .font_weight(FontWeight::BLACK)
                            .child(page.title(cx)),
                    )
                    .child(
                        div()
                            .mt(px(2.))
                            .text_size(px(14.))
                            .text_color(theme.c("ink-3"))
                            .child(page.hint(cx)),
                    ),
            );

        let content = div()
            .id("prefs-content")
            .flex_1()
            .min_w_0()
            .overflow_y_scroll()
            .track_scroll(&self.scroll)
            .px(px(28.))
            .pt(px(8.))
            .pb(px(24.))
            .child(page_head)
            .children(self.current());

        let foot = div()
            .flex()
            .flex_none()
            .justify_end()
            .px(px(20.))
            .py(px(12.))
            .border_t_1()
            .border_color(theme.c("line"))
            .child(widgets::button(
                "prefs-done",
                t(cx, "preferences.done"),
                Kind::Plain,
                Size::Normal,
                None,
                false,
                &theme,
                |_, w, cx| close(w, cx),
            ));

        widgets::base(&theme)
            .size_full()
            .flex()
            .flex_col()
            .min_h_0()
            .child(head)
            .child(div().flex().flex_1().min_h_0().child(nav).child(content))
            .child(foot)
    }
}
