//! Suggest Away when the operator steps away from the Mac, and Available
//! when they come back. Never a switch: the main window shows the
//! suggestion ([`super::suggest`]) and only the operator's click changes the
//! state.
//!
//! The readings are `ioreg`'s, which needs no permission and raises no
//! dialog: `HIDIdleTime` (nanoseconds since the last keyboard or pointer
//! event, from `IOHIDSystem`) and `IOConsoleLocked` / the console user's
//! `CGSSessionScreenIsLocked` (the root's `IOConsoleUsers`).

use std::process::Command;

/// Away after this long without a key or a pointer move.
pub const AWAY_AFTER_MS: u64 = 10 * 60_000;
/// Back when the last input is this recent and the screen is unlocked.
pub const BACK_WITHIN_MS: u64 = 60_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Suggest {
    Away,
    Available,
}

impl Suggest {
    pub fn state(self) -> &'static str {
        match self {
            Suggest::Away => "away",
            Suggest::Available => "available",
        }
    }
}

/// One absence at a time: Away is suggested once when it starts, and
/// Available once when it ends, if the operator is in Away by then.
#[derive(Debug, Default)]
pub struct AwayWatch {
    away: bool,
    suggested: bool,
}

impl AwayWatch {
    pub fn step(
        &mut self,
        idle_ms: u64,
        locked: bool,
        state: &str,
        enabled: bool,
    ) -> Option<Suggest> {
        let quiet_state = matches!(state, "away" | "done");
        if locked || idle_ms >= AWAY_AFTER_MS {
            self.away = true;
            if enabled && !quiet_state && !self.suggested {
                self.suggested = true;
                return Some(Suggest::Away);
            }
            return None;
        }
        if idle_ms < BACK_WITHIN_MS && self.away {
            self.away = false;
            self.suggested = false;
            if enabled && state == "away" {
                return Some(Suggest::Available);
            }
        }
        None
    }
}

/// `"HIDIdleTime" = <ns>` from `ioreg -c IOHIDSystem`, in milliseconds.
pub fn parse_idle_ms(ioreg: &str) -> Option<u64> {
    let line = ioreg.lines().find(|l| l.contains("\"HIDIdleTime\""))?;
    let ns: u64 = line.split('=').nth(1)?.trim().parse().ok()?;
    Some(ns / 1_000_000)
}

/// Whether the screen is locked, from `ioreg -n Root -d1`.
pub fn parse_locked(ioreg: &str) -> Option<bool> {
    if let Some(at) = ioreg.find("\"CGSSessionScreenIsLocked\"=") {
        return Some(ioreg[at..].starts_with("\"CGSSessionScreenIsLocked\"=Yes"));
    }
    let line = ioreg.lines().find(|l| l.contains("\"IOConsoleLocked\""))?;
    Some(line.split('=').nth(1)?.trim().starts_with("Yes"))
}

/// The two readings, from the real `ioreg`. `None` when it cannot answer.
pub fn read_now() -> Option<(u64, bool)> {
    let run = |args: &[&str]| {
        Command::new("/usr/sbin/ioreg")
            .args(args)
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
    };
    let idle = parse_idle_ms(&run(&["-c", "IOHIDSystem", "-d", "4"])?)?;
    let locked = run(&["-n", "Root", "-d", "1"])
        .and_then(|t| parse_locked(&t))
        .unwrap_or(false);
    Some((idle, locked))
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIN: u64 = 60_000;

    #[test]
    fn ten_minutes_idle_suggests_away_once_and_coming_back_suggests_available() {
        let mut w = AwayWatch::default();
        assert_eq!(
            w.step(5_000, false, "available", true),
            None,
            "at the keyboard"
        );
        assert_eq!(w.step(9 * MIN, false, "available", true), None, "not yet");
        assert_eq!(
            w.step(10 * MIN, false, "available", true),
            Some(Suggest::Away)
        );
        assert_eq!(
            w.step(25 * MIN, false, "available", true),
            None,
            "once per absence"
        );
        // They accepted: the state is Away now. Back at the keyboard.
        assert_eq!(w.step(2_000, false, "away", true), Some(Suggest::Available));
        assert_eq!(w.step(4_000, false, "away", true), None, "once per return");
        // A second absence suggests again.
        assert_eq!(
            w.step(11 * MIN, false, "available", true),
            Some(Suggest::Away)
        );
    }

    #[test]
    fn a_locked_screen_is_away_at_once() {
        let mut w = AwayWatch::default();
        assert_eq!(w.step(3_000, true, "focused", true), Some(Suggest::Away));
    }

    #[test]
    fn already_away_or_done_is_never_told_to_leave() {
        for state in ["away", "done"] {
            let mut w = AwayWatch::default();
            assert_eq!(w.step(30 * MIN, false, state, true), None, "{state}");
            assert_eq!(w.step(30 * MIN, true, state, true), None, "{state}");
        }
        // Back after an absence spent in Done for the day: nothing to suggest.
        let mut w = AwayWatch::default();
        w.step(30 * MIN, false, "done", true);
        assert_eq!(w.step(1_000, false, "done", true), None);
    }

    #[test]
    fn coming_back_suggests_available_only_when_in_away() {
        let mut w = AwayWatch::default();
        assert_eq!(
            w.step(12 * MIN, false, "available", true),
            Some(Suggest::Away)
        );
        // They said no: still Available when they return.
        assert_eq!(w.step(1_000, false, "available", true), None);
        // They set Away themselves and left: on return, Available is offered.
        let mut w = AwayWatch::default();
        assert_eq!(w.step(20 * MIN, false, "away", true), None);
        assert_eq!(w.step(1_000, false, "away", true), Some(Suggest::Available));
    }

    #[test]
    fn turned_off_in_preferences_it_says_nothing() {
        let mut w = AwayWatch::default();
        assert_eq!(w.step(30 * MIN, true, "available", false), None);
        assert_eq!(w.step(1_000, false, "away", false), None);
    }

    #[test]
    fn the_idle_and_lock_readings_come_from_ioreg_text() {
        let hid = r#"    | |   "HIDIdleTime" = 112561680125
    | |   "HIDParameters" = {}"#;
        assert_eq!(parse_idle_ms(hid), Some(112_561));
        assert_eq!(parse_idle_ms("nothing here"), None);
        let root = r#"  "IOConsoleUsers" = ({"kCGSSessionOnConsoleKey"=Yes,"CGSSessionScreenIsLocked"=Yes,"kCGSSessionUserNameKey"="p"})"#;
        assert_eq!(parse_locked(root), Some(true));
        assert_eq!(parse_locked(r#""IOConsoleLocked" = No"#), Some(false));
        assert_eq!(parse_locked(r#""IOConsoleLocked" = Yes"#), Some(true));
        assert_eq!(parse_locked("{}"), None);
    }
}
