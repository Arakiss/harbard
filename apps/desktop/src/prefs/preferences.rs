//! The Preferences dialog (`preferences.tsx`): a 920 × 720 modal with a
//! left nav of five pages, each opening with its name and a summary.
//!
//! ```text
//! ┌ Preferences ─────────────────────────────── [x] ┐  head 56
//! │ nav 200 │ page name, what it holds               │
//! │ ◦ Appear│ section   section   …   (scrolls)      │
//! │ ◦ Genera│                                        │
//! ├─────────┴──────────────────────────────── [Done]┤  foot
//! ```

use super::appearance::AppearancePage;
use super::general::{GeneralPage, NotificationsPage};
use super::pairing::PairingPage;
use super::updates_page::UpdatesPage;
use super::widgets::{self, Kind, Size};
use crate::i18n::t;
use crate::theme::Theme;
use crate::ui::{self, icon_size, radius, row, size, space, weight};
use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyView, App, AppContext, Context, Entity, Global, InteractiveElement, IntoElement,
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
        DIALOG_W,
        Some(DIALOG_H),
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
        let head = widgets::dialog_head_closing(t(cx, "preferences.preferences"), &theme, close);

        // The nav's labels start on the header title's line: MD + LG is the gutter.
        let nav = div()
            .w(px(NAV_W))
            .flex_none()
            .flex()
            .flex_col()
            .gap(px(space::HAIR))
            .px(px(space::MD))
            .py(px(space::LG))
            .border_r_1()
            .border_color(theme.c("line"))
            .children(Page::ALL.into_iter().map(|p| {
                let on = p == page;
                let hover = theme.c("hover");
                let (ink, mark) = if on {
                    (theme.c("accent-ink"), theme.c("accent-ink"))
                } else {
                    (theme.c("ink"), theme.c("ink-3"))
                };
                widgets::focusable(
                    div().id(SharedString::from(format!("prefs-nav-{p:?}"))),
                    &theme,
                    true,
                )
                .flex()
                .flex_none()
                .items_center()
                .gap(px(space::MD))
                .h(px(row::NAV))
                .px(px(space::LG))
                .rounded(px(radius::BASE))
                .cursor_pointer()
                .when(on, |d| d.bg(theme.c("accent")))
                .when(!on, |d| d.hover(move |s| s.bg(hover)))
                .child(ui::icon(p.icon(), icon_size::ROW, mark))
                .child(
                    ui::text(
                        p.title(cx),
                        size::UI,
                        if on { weight::STRONG } else { weight::REGULAR },
                        ink,
                    )
                    .truncate(),
                )
                .on_click(cx.listener(move |this, _, window, cx| this.show(p, window, cx)))
            }));

        // The page's name and what it holds; the dialog's one title is above.
        let page_head = div()
            .flex()
            .flex_col()
            .gap(px(space::HAIR))
            .pb(px(space::SECTION))
            .child(ui::text(
                page.title(cx),
                size::BODY,
                weight::STRONG,
                theme.c("ink"),
            ))
            .child(ui::text(
                page.hint(cx),
                size::SMALL,
                weight::REGULAR,
                theme.c("ink-3"),
            ));

        let content = div()
            .id("prefs-content")
            .flex_1()
            .min_w_0()
            .overflow_y_scroll()
            .track_scroll(&self.scroll)
            .px(px(space::GUTTER))
            .pt(px(space::XL))
            .pb(px(space::SECTION))
            .child(page_head)
            .children(self.current());

        let foot = widgets::dialog_foot(&theme).child(widgets::button(
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

/// The dialog and its nav column.
const DIALOG_W: f32 = 920.;
const DIALOG_H: f32 = 720.;
const NAV_W: f32 = 200.;
