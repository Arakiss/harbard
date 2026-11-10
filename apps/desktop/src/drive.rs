//! A test driver for development: with `HARBARD_APP_DRIVE=<file>` set, the
//! app reads commands appended to that file and feeds them to the main
//! window as input, so a script can click, hover, scroll and type in a
//! window that is not in front (macOS does not deliver background mouse
//! events to it). Without the variable nothing here runs.
//!
//! One command per line, coordinates in window points:
//! `move x y`, `click x y`, `dclick x y`, `rclick x y`, `scroll x y dy`,
//! `key <keystroke>` (`escape`, `enter`, `cmd-enter`, `down`), `type <text>`.

use gpui::{
    App, AsyncApp, Keystroke, Modifiers, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, PlatformInput,
    ScrollDelta, ScrollEvent, TouchPhase, point, px,
};
use std::time::Duration;

pub fn init(cx: &mut App) {
    let Some(path) = std::env::var_os("HARBARD_APP_DRIVE") else { return };
    let path = std::path::PathBuf::from(path);
    let _ = std::fs::write(&path, "");
    cx.spawn(async move |cx: &mut AsyncApp| {
        let mut done = 0usize;
        loop {
            cx.background_executor().timer(Duration::from_millis(150)).await;
            let Ok(text) = std::fs::read_to_string(&path) else { continue };
            let lines: Vec<String> = text.lines().map(str::to_string).collect();
            for line in lines.iter().skip(done) {
                let line = line.clone();
                cx.update(|cx| run(&line, cx));
                cx.background_executor().timer(Duration::from_millis(60)).await;
            }
            done = lines.len();
        }
    })
    .detach();
}

fn run(line: &str, cx: &mut App) {
    let mut words = line.split_whitespace();
    let Some(cmd) = words.next() else { return };
    let rest: Vec<&str> = words.collect();
    let num = |i: usize| rest.get(i).and_then(|s| s.parse::<f32>().ok()).unwrap_or(0.);
    let Some(window) = cx.windows().into_iter().next() else { return };
    let at = point(px(num(0)), px(num(1)));
    let events: Vec<PlatformInput> = match cmd {
        "move" => vec![PlatformInput::MouseMove(MouseMoveEvent {
            position: at,
            pressed_button: None,
            modifiers: Modifiers::default(),
        })],
        "click" | "dclick" | "rclick" => {
            let button = if cmd == "rclick" { MouseButton::Right } else { MouseButton::Left };
            let counts: &[usize] = if cmd == "dclick" { &[1, 2] } else { &[1] };
            let mut out = vec![PlatformInput::MouseMove(MouseMoveEvent {
                position: at,
                pressed_button: None,
                modifiers: Modifiers::default(),
            })];
            for &n in counts {
                out.push(PlatformInput::MouseDown(MouseDownEvent {
                    button,
                    position: at,
                    modifiers: Modifiers::default(),
                    click_count: n,
                    first_mouse: false,
                }));
                out.push(PlatformInput::MouseUp(MouseUpEvent {
                    button,
                    position: at,
                    modifiers: Modifiers::default(),
                    click_count: n,
                }));
            }
            out
        }
        "scroll" => vec![PlatformInput::Scroll(ScrollEvent {
            position: at,
            delta: ScrollDelta::Pixels(point(px(0.), px(-num(2)))),
            modifiers: Modifiers::default(),
            touch_phase: TouchPhase::Moved,
            ..Default::default()
        })],
        "key" => {
            let source: &'static str = Box::leak(rest.join(" ").into_boxed_str());
            if let Ok(k) = Keystroke::parse(source) {
                let _ = window.update(cx, |_, window, cx| window.dispatch_keystroke(k, cx));
            }
            return;
        }
        "type" => {
            let text = line.split_once(' ').map_or("", |(_, t)| t).to_string();
            for c in text.chars() {
                let key = if c == ' ' { "space".to_string() } else { c.to_string() };
                let source: &'static str = Box::leak(key.into_boxed_str());
                if let Ok(mut k) = Keystroke::parse(source) {
                    k.key_char = Some(c.to_string());
                    let _ = window.update(cx, |_, window, cx| window.dispatch_keystroke(k, cx));
                }
            }
            return;
        }
        _ => return,
    };
    let _ = window.update(cx, |_, window, cx| {
        for e in events {
            window.dispatch_event(e, cx);
        }
        window.refresh();
    });
}
