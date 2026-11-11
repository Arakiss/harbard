//! A dev sheet of every robot: `harbard-app --robot-gallery` opens it
//! instead of the workspace. Every model at the list sizes, every face at
//! a tiny and a large size, the dots, and the agents of the current read
//! with their live faces and status lines.

use super::{Face, Pick, Ring, robot, status_line};
use crate::desk::Desk;
use crate::theme::Theme;
use crate::ui::{space, weight};
use gpui::{
    App, AppContext, Bounds, Context, InteractiveElement, IntoElement, ParentElement, Render,
    ScrollHandle, StatefulInteractiveElement, Styled, TitlebarOptions, Window, WindowBounds,
    WindowOptions, div, point, px, size,
};

pub fn wanted() -> bool {
    std::env::args().any(|a| a == "--robot-gallery")
}

pub fn open(cx: &mut App) {
    let bounds = Bounds::centered(None, size(px(1180.), px(860.)), cx);
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        titlebar: Some(TitlebarOptions {
            title: Some("robots".into()),
            appears_transparent: true,
            traffic_light_position: Some(point(px(14.), px(18.))),
        }),
        // Above other windows and without taking focus, so its robots stay
        // visible (a covered window draws nothing) while it is measured.
        kind: gpui::WindowKind::PopUp,
        focus: false,
        ..Default::default()
    };
    gpui_kit::open_window(options, cx, |window, cx| {
        cx.new(|cx| Gallery::new(window, cx))
    })
    .expect("open the gallery");
}

struct Gallery {
    scroll: ScrollHandle,
}

impl Gallery {
    fn new(_window: &mut Window, cx: &mut Context<Self>) -> Self {
        cx.observe(&Desk::get(cx), |_, _, cx| cx.notify()).detach();
        cx.observe_global::<Theme>(|_, cx| cx.notify()).detach();
        Gallery {
            scroll: ScrollHandle::new(),
        }
    }
}

fn label(text: impl Into<gpui::SharedString>, theme: &Theme) -> impl IntoElement {
    crate::ui::text(
        text,
        crate::ui::size::META,
        weight::STRONG,
        theme.c("ink-3"),
    )
}

impl Render for Gallery {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.global::<Theme>().clone();
        let mut sheet = div()
            .flex_none()
            .flex()
            .flex_col()
            .gap(px(space::GUTTER))
            .p(px(space::SECTION))
            .pt(px(44.));

        // The live agents, as the Home sidebar lists them.
        let agents = Desk::get(cx)
            .read(cx)
            .view
            .as_ref()
            .map(|v| v.agents.clone())
            .unwrap_or_default();
        let mut live = div()
            .flex()
            .flex_col()
            .w(px(300.))
            .bg(theme.c("sidebar"))
            .py(px(space::SM));
        for a in agents.iter().take(14) {
            let idle = a.activity.state == "normal" || a.activity.state.is_empty();
            let avatar = robot(
                crate::derive::robot_name(a).to_string(),
                if idle { 24.0 } else { 32.0 },
            )
            .agent(a)
            .dot(true)
            .ring(Ring::Sidebar);
            live = live.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(space::MD))
                    .px(px(space::XL))
                    .py(px(space::XS))
                    .min_h(px(44.))
                    .child(avatar)
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .flex_1()
                            .min_w_0()
                            .child(crate::ui::text(
                                crate::derive::agent_name(a, cx),
                                15.0,
                                weight::STRONG,
                                theme.c("sidebar-ink"),
                            ))
                            .child(
                                status_line(a)
                                    .color(theme.c("sidebar-ink-2"))
                                    .long_color(theme.c("sidebar-ink")),
                            ),
                    ),
            );
        }
        sheet = sheet.child(label("live agents", &theme)).child(live);

        // Every face, tiny and large.
        for s in [32.0, 36.0, 64.0, 104.0] {
            sheet = sheet.child(label(format!("faces at {s}"), &theme));
            let mut row = div().flex().flex_wrap().gap(px(space::LG));
            for (i, f) in Face::ALL.into_iter().enumerate() {
                let name = super::identity::ROBOT_MODELS[i % 12].id;
                row = row.child(
                    div()
                        .flex()
                        .flex_col()
                        .items_center()
                        .gap(px(space::XS))
                        .child(
                            robot(format!("{name}-{}", f.id()), s)
                                .pick(Pick {
                                    model: Some(name),
                                    ..Pick::default()
                                })
                                .face(f),
                        )
                        .child(label(f.id(), &theme)),
                );
            }
            sheet = sheet.child(row);
        }

        // Dots on each surround.
        let mut dots = div()
            .flex()
            .gap(px(space::XL))
            .p(px(space::MD))
            .bg(theme.c("sidebar"));
        for tone in ["green", "amber", "red", "grey"] {
            dots = dots.child(robot("keychain", 32.0).tone(tone).ring(Ring::Sidebar));
        }
        sheet = sheet
            .child(label("dots on the sidebar", &theme))
            .child(dots);
        let others = div()
            .flex()
            .gap(px(space::XL))
            .child(crate::ui::avatar::monogram_avatar(
                "harbard",
                None,
                None,
                36.0,
                Some("green"),
                cx,
            ))
            .child(crate::ui::avatar::lead_avatar(36.0, cx))
            .child(crate::ui::avatar::operator_avatar("PA", 36.0, cx))
            .child(crate::ui::avatar::operator_avatar_with(
                "Petru",
                "PA",
                36.0,
                Some("available"),
                cx,
            ))
            .child(crate::ui::avatar::operator_avatar_with(
                "Petru",
                "PA",
                36.0,
                Some("focused"),
                cx,
            ))
            .child(crate::ui::avatar::operator_avatar_with(
                "Petru",
                "PA",
                36.0,
                Some("do-not-disturb"),
                cx,
            ))
            .child(crate::ui::avatar::name_avatar("someone", 36.0, cx));
        sheet = sheet.child(label("avatars", &theme)).child(others);

        // Every model at the sizes the console uses.
        sheet = sheet.child(label(format!("models · season {:?}", theme.season), &theme));
        for m in super::identity::ROBOT_MODELS {
            let mut row = div()
                .flex_none()
                .flex()
                .items_end()
                .gap(px(space::LG))
                .child(div().w(px(80.)).child(label(m.id, &theme)));
            for s in [20.0, 24.0, 32.0, 36.0, 48.0, 64.0, 104.0] {
                row = row.child(robot(m.id, s).pick(Pick {
                    model: Some(m.id),
                    ..Pick::default()
                }));
            }
            sheet = sheet.child(row);
        }

        div()
            .id("robot-gallery")
            .size_full()
            .flex()
            .flex_col()
            .bg(theme.c("surface"))
            .text_color(theme.c("ink"))
            .overflow_y_scroll()
            .track_scroll(&self.scroll)
            .child(sheet)
    }
}
