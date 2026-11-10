//! Seasons: a built-in theme that takes over the theme layer between two
//! dates, without touching the theme the operator chose.
//!
//! A season is data ([`SEASONS`]): an id, the built-in theme it paints and
//! an inclusive window of month and day that may wrap the new year. The
//! operator's preference (`season` in `workspace.json`) is `auto` (the
//! default: on inside the window, by the operator's local date), `on` or
//! `off`; decorations (`decorations`) are a separate switch, on unless
//! stored `false`.

use serde::Serialize;
use time::Date;

/// One season: the theme it paints from `from` to `to`, both included.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Season {
    pub id: &'static str,
    /// A built-in theme id whose spec names this season.
    pub theme: &'static str,
    /// `(month, day)`, inclusive.
    pub from: (u8, u8),
    pub to: (u8, u8),
}

/// Every season, in the order they come in a year that starts on 1 January.
pub const SEASONS: &[Season] = &[Season {
    id: "christmas",
    theme: "christmas",
    from: (12, 1),
    to: (1, 6),
}];

impl Season {
    /// Whether `(month, day)` falls inside the window, which may wrap the year.
    pub fn contains(&self, month: u8, day: u8) -> bool {
        let d = (month, day);
        if self.from <= self.to {
            self.from <= d && d <= self.to
        } else {
            d >= self.from || d <= self.to
        }
    }

    /// Days from `(month, day)` to the window's first day, in a 366-day year.
    fn days_until(&self, month: u8, day: u8) -> u32 {
        let ordinal = |(m, d): (u8, u8)| u32::from(m - 1) * 31 + u32::from(d);
        let (now, start) = (ordinal((month, day)), ordinal(self.from));
        if start >= now {
            start - now
        } else {
            start + 12 * 31 - now
        }
    }
}

/// The operator's choice.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Choice {
    Auto,
    On,
    Off,
}

impl Choice {
    pub fn parse(raw: &str) -> Option<Choice> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "auto" => Some(Choice::Auto),
            "on" => Some(Choice::On),
            "off" => Some(Choice::Off),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Choice::Auto => "auto",
            Choice::On => "on",
            Choice::Off => "off",
        }
    }
}

/// The season the console is told about: the one in its window today, or
/// the next to come. `on` turns that one on outside its dates.
pub fn current(month: u8, day: u8) -> Option<&'static Season> {
    SEASONS
        .iter()
        .find(|s| s.contains(month, day))
        .or_else(|| SEASONS.iter().min_by_key(|s| s.days_until(month, day)))
}

/// What `/api/v1/theme` says about the season.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SeasonState {
    pub id: &'static str,
    /// The built-in theme that replaces the theme layer while active.
    pub theme: &'static str,
    /// The stored choice, `auto` when none or unreadable.
    pub choice: Choice,
    /// Whether the season paints now.
    pub active: bool,
    /// Today inside the window, whatever the choice.
    pub in_window: bool,
    /// Snow, lights and the rest, while active. On unless stored false.
    pub decorations: bool,
    /// `MM-DD`, inclusive.
    pub from: String,
    pub to: String,
    /// The operator's local date the answer was computed for, `YYYY-MM-DD`.
    pub today: String,
}

/// The season state for `today` and the stored choice.
pub fn state(today: Date, choice: Option<&str>, decorations: Option<bool>) -> Option<SeasonState> {
    let (month, day) = (today.month() as u8, today.day());
    let season = current(month, day)?;
    let choice = choice.and_then(Choice::parse).unwrap_or(Choice::Auto);
    let in_window = season.contains(month, day);
    let active = match choice {
        Choice::Auto => in_window,
        Choice::On => true,
        Choice::Off => false,
    };
    Some(SeasonState {
        id: season.id,
        theme: season.theme,
        choice,
        active,
        in_window,
        decorations: decorations.unwrap_or(true),
        from: format!("{:02}-{:02}", season.from.0, season.from.1),
        to: format!("{:02}-{:02}", season.to.0, season.to.1),
        today: format!(
            "{:04}-{:02}-{:02}",
            today.year(),
            today.month() as u8,
            today.day()
        ),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use time::Month;

    fn day(y: i32, m: u8, d: u8) -> Date {
        Date::from_calendar_date(y, Month::try_from(m).unwrap(), d).unwrap()
    }

    #[test]
    fn christmas_runs_from_the_first_of_december_to_twelfth_night_inclusive() {
        let on = |m, d| state(day(2026, m, d), None, None).unwrap().active;
        assert!(!on(11, 30));
        assert!(on(12, 1));
        assert!(on(12, 25));
        assert!(on(12, 31));
        assert!(on(1, 1));
        assert!(on(1, 6));
        assert!(!on(1, 7));
        assert!(!on(7, 15));
    }

    #[test]
    fn on_and_off_override_the_dates_and_auto_is_the_default() {
        let s = state(day(2026, 11, 10), Some("on"), None).unwrap();
        assert!(s.active && !s.in_window);
        assert_eq!((s.id, s.choice), ("christmas", Choice::On));
        let s = state(day(2026, 12, 24), Some("off"), Some(true)).unwrap();
        assert!(!s.active && s.in_window && s.decorations);
        let s = state(day(2026, 12, 24), Some("nonsense"), Some(false)).unwrap();
        assert!(s.active && !s.decorations);
        assert_eq!(s.choice, Choice::Auto);
        assert_eq!((s.from.as_str(), s.to.as_str()), ("12-01", "01-06"));
        assert_eq!(s.today, "2026-12-24");
    }

    #[test]
    fn every_season_paints_a_seasonal_builtin() {
        for s in SEASONS {
            let spec = super::super::builtin(s.theme).expect("built-in");
            assert_eq!(spec.season, Some(s.id), "{}", s.id);
        }
        for spec in super::super::builtins() {
            if let Some(id) = spec.season {
                assert!(SEASONS.iter().any(|s| s.id == id), "{id}");
            }
        }
    }

    #[test]
    fn a_window_that_does_not_wrap_also_works() {
        let s = Season {
            id: "spring",
            theme: "spring",
            from: (3, 20),
            to: (4, 10),
        };
        assert!(s.contains(3, 20) && s.contains(4, 10) && s.contains(4, 1));
        assert!(!s.contains(3, 19) && !s.contains(4, 11) && !s.contains(12, 25));
    }
}
