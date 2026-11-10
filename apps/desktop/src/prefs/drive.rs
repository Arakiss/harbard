//! A test driver: `HARBARD_APP_DRIVE` plays pointer and key input into the
//! main window from inside the app, through GPUI's own dispatch (hit
//! testing, hover, focus, the input handler), so a dev build can be
//! exercised and captured without moving anybody's mouse or taking focus.
//!
//! Steps are separated by `;`, coordinates are window points:
//! `wait 800; move 735 510; click 343 230; type hello; key escape; scroll 800 500 -400`.

use gpui::{
    App, Keystroke, Modifiers, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, PlatformInput, ScrollDelta,
    ScrollEvent, Window, point, px,
};
use std::time::Duration;

pub fn start(script: String, cx: &mut App) {
    cx.spawn(async move |cx| {
        for step in script.split(';').map(str::trim).filter(|s| !s.is_empty()) {
            let mut words = step.split_whitespace();
            let verb = words.next().unwrap_or_default();
            let rest: Vec<&str> = words.collect();
            let num = |i: usize| rest.get(i).and_then(|v| v.parse::<f32>().ok()).unwrap_or(0.);
            if verb == "wait" {
                cx.background_executor()
                    .timer(Duration::from_millis(num(0) as u64))
                    .await;
                continue;
            }
            let step = step.to_string();
            cx.update(|cx| {
                let Some(window) = cx.windows().into_iter().next() else {
                    return;
                };
                let _ = window.update(cx, |_, window, cx| play(&step, window, cx));
            });
            cx.background_executor().timer(Duration::from_millis(120)).await;
        }
    })
    .detach();
}

fn play(step: &str, window: &mut Window, cx: &mut App) {
    let mut words = step.split_whitespace();
    let verb = words.next().unwrap_or_default();
    let rest: Vec<&str> = words.collect();
    let num = |i: usize| rest.get(i).and_then(|v| v.parse::<f32>().ok()).unwrap_or(0.);
    let at = point(px(num(0)), px(num(1)));
    match verb {
        "move" => {
            window.dispatch_event(
                PlatformInput::MouseMove(MouseMoveEvent {
                    position: at,
                    pressed_button: None,
                    modifiers: Modifiers::default(),
                }),
                cx,
            );
        }
        "click" => {
            window.dispatch_event(
                PlatformInput::MouseMove(MouseMoveEvent {
                    position: at,
                    pressed_button: None,
                    modifiers: Modifiers::default(),
                }),
                cx,
            );
            window.dispatch_event(
                PlatformInput::MouseDown(MouseDownEvent {
                    button: MouseButton::Left,
                    position: at,
                    modifiers: Modifiers::default(),
                    click_count: 1,
                    first_mouse: false,
                }),
                cx,
            );
            window.dispatch_event(
                PlatformInput::MouseUp(MouseUpEvent {
                    button: MouseButton::Left,
                    position: at,
                    modifiers: Modifiers::default(),
                    click_count: 1,
                }),
                cx,
            );
        }
        "scroll" => {
            window.dispatch_event(
                PlatformInput::Scroll(ScrollEvent {
                    position: at,
                    delta: ScrollDelta::Pixels(point(px(0.), px(num(2)))),
                    modifiers: Modifiers::default(),
                    ..Default::default()
                }),
                cx,
            );
        }
        "type" => {
            let text = step.splitn(2, ' ').nth(1).unwrap_or_default();
            for ch in text.chars() {
                let s = ch.to_string();
                let keystroke = Keystroke {
                    modifiers: Modifiers::default(),
                    key: s.to_lowercase(),
                    key_char: Some(s),
                    ..Default::default()
                };
                window.dispatch_keystroke(keystroke, cx);
            }
        }
        "key" => {
            let keystroke = Keystroke {
                modifiers: Modifiers::default(),
                key: rest.first().copied().unwrap_or_default().to_string(),
                key_char: None,
                ..Default::default()
            };
            window.dispatch_keystroke(keystroke, cx);
        }
        _ => {}
    }
    window.refresh();
}
