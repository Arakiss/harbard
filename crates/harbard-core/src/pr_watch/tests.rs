use super::*;
use crate::test_util::TempHome;
use std::cell::RefCell;

const REPO: &str = "/work/northwind";
const SECOND: u64 = 1000;
const INTERVAL: u64 = MIN_POLL_INTERVAL_SECS * SECOND;

/// A scripted GitHub: answers from a table and records every question.
struct Scripted {
    answers: RefCell<BTreeMap<u64, Result<PrState, String>>>,
    asked: RefCell<Vec<u64>>,
}

impl Scripted {
    fn new(answers: &[(u64, Result<PrState, String>)]) -> Self {
        Self {
            answers: RefCell::new(answers.iter().cloned().collect()),
            asked: RefCell::new(Vec::new()),
        }
    }

    fn answer(&self, number: u64, state: Result<PrState, String>) {
        self.answers.borrow_mut().insert(number, state);
    }

    fn asked(&self) -> Vec<u64> {
        self.asked.borrow().clone()
    }
}

impl PrSource for Scripted {
    fn state(&self, _repo: &str, number: u64) -> Result<PrState, String> {
        self.asked.borrow_mut().push(number);
        self.answers
            .borrow()
            .get(&number)
            .cloned()
            .unwrap_or(Ok(PrState::Open))
    }
}

#[test]
fn a_wanted_pr_is_answered_on_the_next_turn_and_read_by_everyone() {
    let home = TempHome::new();
    let watch = PrWatch::at(home.path());
    let source = Scripted::new(&[(2258, Ok(PrState::Merged))]);
    assert_eq!(watch.want(REPO, 2258, 0).unwrap(), None);
    let outcome = watch.tick(&source, &[], 0, 1).unwrap();
    assert_eq!(
        outcome,
        TickOutcome::Polled {
            asked: 1,
            failed: false
        }
    );
    let elsewhere = PrWatch::at(home.path()).snapshot();
    assert_eq!(elsewhere.state(REPO, 2258), Some(PrState::Merged));
    assert!(elsewhere.settled(REPO, 2258));
}

#[test]
fn only_one_process_polls_per_interval() {
    let home = TempHome::new();
    let first = PrWatch::at(home.path());
    let second = PrWatch::at(home.path());
    let source = Scripted::new(&[]);
    first.want(REPO, 1, 0).unwrap();
    first.tick(&source, &[], 0, 1).unwrap();
    assert_eq!(
        second.tick(&source, &[], 0, 1 + INTERVAL - 1).unwrap(),
        TickOutcome::NotDue
    );
    assert_eq!(source.asked(), vec![1]);
    second.tick(&source, &[], 0, 1 + INTERVAL).unwrap();
    assert_eq!(source.asked(), vec![1, 1]);
}

#[test]
fn the_interval_never_drops_below_the_floor() {
    let home = TempHome::new();
    let watch = PrWatch::at(home.path());
    let source = Scripted::new(&[]);
    watch.want(REPO, 1, 0).unwrap();
    watch.tick(&source, &[], 5, 1).unwrap();
    assert_eq!(
        watch.tick(&source, &[], 5, 1 + 60 * SECOND).unwrap(),
        TickOutcome::NotDue
    );
}

#[test]
fn settled_prs_are_never_asked_again() {
    let home = TempHome::new();
    let watch = PrWatch::at(home.path());
    let source = Scripted::new(&[(1, Ok(PrState::Closed))]);
    watch.want(REPO, 1, 0).unwrap();
    watch.tick(&source, &[], 0, 1).unwrap();
    assert_eq!(
        watch.tick(&source, &[], 0, 1 + INTERVAL).unwrap(),
        TickOutcome::Idle
    );
    assert_eq!(source.asked(), vec![1]);
}

#[test]
fn a_failure_backs_off_beyond_the_interval_and_then_recovers() {
    let home = TempHome::new();
    let watch = PrWatch::at(home.path());
    let source = Scripted::new(&[(1, Err("rate limited".into()))]);
    watch.want(REPO, 1, 0).unwrap();
    let failed = watch.tick(&source, &[], 0, 1).unwrap();
    assert_eq!(
        failed,
        TickOutcome::Polled {
            asked: 1,
            failed: true
        }
    );
    assert!(matches!(
        watch.tick(&source, &[], 0, 1 + INTERVAL).unwrap(),
        TickOutcome::BackingOff { .. }
    ));
    source.answer(1, Ok(PrState::Merged));
    let recovered = watch.tick(&source, &[], 0, 1 + 2 * INTERVAL).unwrap();
    assert_eq!(
        recovered,
        TickOutcome::Polled {
            asked: 1,
            failed: false
        }
    );
}

#[test]
fn a_turn_asks_a_bounded_batch_oldest_answer_first() {
    let home = TempHome::new();
    let watch = PrWatch::at(home.path());
    let source = Scripted::new(&[]);
    let total = MAX_REQUESTS_PER_TURN as u64 + 2;
    for number in 1..=total {
        watch.want(REPO, number, 0).unwrap();
    }
    watch.tick(&source, &[], 0, 1).unwrap();
    assert_eq!(source.asked().len(), MAX_REQUESTS_PER_TURN);
    watch.tick(&source, &[], 0, 1 + INTERVAL).unwrap();
    let second_turn = &source.asked()[MAX_REQUESTS_PER_TURN..];
    assert_eq!(
        &second_turn[..2],
        &[total - 1, total],
        "never-answered PRs go before re-asking answered ones"
    );
}

#[test]
fn the_hourly_budget_stops_polling_until_the_window_turns() {
    let home = TempHome::new();
    let watch = PrWatch::at(home.path());
    let source = Scripted::new(&[]);
    for number in 1..=MAX_REQUESTS_PER_TURN as u64 {
        watch.want(REPO, number, 0).unwrap();
    }
    let paid_turns = u64::from(HOURLY_REQUEST_BUDGET) / MAX_REQUESTS_PER_TURN as u64;
    assert!(
        paid_turns * INTERVAL < HOUR_MS,
        "the budget must bind before the hour ends for this test to mean anything"
    );
    let start = 1;
    for turn in 0..paid_turns {
        watch
            .tick(&source, &[], 0, start + turn * INTERVAL)
            .unwrap();
    }
    assert_eq!(source.asked().len(), HOURLY_REQUEST_BUDGET as usize);
    assert_eq!(
        watch
            .tick(&source, &[], 0, start + paid_turns * INTERVAL)
            .unwrap(),
        TickOutcome::BudgetSpent
    );
    watch.tick(&source, &[], 0, start + HOUR_MS).unwrap();
    assert_eq!(
        source.asked().len(),
        HOURLY_REQUEST_BUDGET as usize + MAX_REQUESTS_PER_TURN
    );
}

#[test]
fn interest_lapses_after_a_day_without_renewal() {
    let home = TempHome::new();
    let watch = PrWatch::at(home.path());
    let source = Scripted::new(&[]);
    watch.want(REPO, 1, 0).unwrap();
    assert_eq!(
        watch.tick(&source, &[], 0, INTEREST_TTL_MS).unwrap(),
        TickOutcome::Idle
    );
    assert!(source.asked().is_empty());
}

#[test]
fn prs_passed_to_tick_are_followed_like_wanted_ones() {
    let home = TempHome::new();
    let watch = PrWatch::at(home.path());
    let source = Scripted::new(&[(7, Ok(PrState::Merged))]);
    watch.tick(&source, &[(REPO.to_string(), 7)], 0, 1).unwrap();
    assert_eq!(watch.snapshot().state(REPO, 7), Some(PrState::Merged));
}

#[cfg(unix)]
mod gh {
    use super::*;
    use std::fs;
    use std::os::unix::fs::PermissionsExt;

    fn script(home: &TempHome, body: &str) -> PathBuf {
        let path = home.path().join("gh-double");
        fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    #[test]
    fn the_gh_source_reads_the_state_gh_prints() {
        let home = TempHome::new();
        let double = script(
            &home,
            r#"[ "$1 $2 $3 $4 $5" = "pr view 2258 --json state" ] || exit 9
echo MERGED"#,
        );
        let source = GhSource::new(&double, Duration::from_secs(5));
        let repo = home.path().display().to_string();
        assert_eq!(source.state(&repo, 2258), Ok(PrState::Merged));
    }

    #[test]
    fn a_failing_or_unexpected_gh_is_an_error_not_a_state() {
        let home = TempHome::new();
        let repo = home.path().display().to_string();
        let failing = GhSource::new(script(&home, "exit 1"), Duration::from_secs(5));
        assert!(failing.state(&repo, 1).is_err());
        let odd = GhSource::new(script(&home, "echo DRAFT"), Duration::from_secs(5));
        assert!(odd.state(&repo, 1).is_err());
    }

    #[test]
    fn a_hung_gh_is_killed_at_the_timeout() {
        let home = TempHome::new();
        let repo = home.path().display().to_string();
        let hung = GhSource::new(script(&home, "sleep 30"), Duration::from_millis(200));
        let started = Instant::now();
        assert!(hung.state(&repo, 1).is_err());
        assert!(started.elapsed() < Duration::from_secs(5));
    }
}
