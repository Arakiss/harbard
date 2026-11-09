//! `harbard update`: check, show what changed, ask, install, follow.

use std::io::{IsTerminal, Write};
use std::time::{Duration, Instant};

use harbard_core::Store;

use super::notes::Change;
use super::run::{self, Run};
use super::{Channel, Check, Prefs};

/// Exit code of `--check` when an update is available.
pub const EXIT_AVAILABLE: i32 = 10;

pub struct Args<'a> {
    pub check: bool,
    pub yes: bool,
    pub channel: Option<&'a str>,
    pub quiet: bool,
    pub detach: bool,
    pub run: Option<&'a str>,
    pub json: bool,
}

pub fn cmd_update(store: &Store, args: &Args<'_>) -> i32 {
    if let Some(id) = args.run {
        return run::execute(store, id);
    }
    let mut prefs = Prefs::load(store);
    if let Some(raw) = args.channel {
        let Some(channel) = Channel::parse(raw) else {
            eprintln!("harbard update: unknown channel {raw:?}; use main or releases");
            return 2;
        };
        if channel != prefs.channel {
            prefs.channel = channel;
            if let Err(e) = prefs.save(store) {
                eprintln!(
                    "harbard update: cannot save {}: {e}",
                    Prefs::path(store).display()
                );
                return 1;
            }
        }
        if !args.quiet && !args.json {
            println!("Update channel: {}", channel_line(channel));
        }
    }
    if run::busy(store) {
        if args.json {
            println!("{}", super::view(store));
            return 0;
        }
        if !args.quiet {
            println!("An update is already running.");
        }
        return if args.detach || args.quiet {
            0
        } else {
            follow(store)
        };
    }

    let check = super::check(store, &prefs);
    if args.json && (args.check || !args.yes) {
        println!("{}", super::view(store));
        return check_exit(&check);
    }
    if args.check {
        if !args.quiet {
            print_check(&check, false);
        }
        return check_exit(&check);
    }
    if !args.json {
        print_check(&check, true);
    }
    if !check.available() {
        return check_exit(&check);
    }
    if !args.yes {
        if !std::io::stdin().is_terminal() {
            println!("\nRun `harbard update --yes` to install it.");
            return EXIT_AVAILABLE;
        }
        print!("\nInstall it now? [Y/n] ");
        let _ = std::io::stdout().flush();
        let mut answer = String::new();
        if std::io::stdin().read_line(&mut answer).is_err() {
            return 1;
        }
        let answer = answer.trim().to_ascii_lowercase();
        if !(answer.is_empty()
            || answer == "y"
            || answer == "yes"
            || answer == "s"
            // A Spanish yes is an answer too: the prompt is read and typed by people.
            || answer == "si"
            || answer == "sí")
        {
            println!("Not installed. `harbard update` whenever you want it.");
            return 0;
        }
    }
    match run::start(store, &prefs) {
        Ok(_) if args.detach => {
            if !args.quiet {
                println!(
                    "Updating in the background. Follow it in the console, or with `harbard update`.\nLog: {}",
                    run::log_path(store).display()
                );
            }
            0
        }
        Ok(_) => follow(store),
        Err(e) => {
            eprintln!("harbard update: {} ({})", e.message, e.code);
            1
        }
    }
}

fn check_exit(check: &Check) -> i32 {
    match check.status.as_str() {
        "available" => EXIT_AVAILABLE,
        "current" => 0,
        _ => 1,
    }
}

fn channel_line(channel: Channel) -> &'static str {
    match channel {
        Channel::Main => "main (every change on main)",
        Channel::Releases => "releases (published versions only)",
    }
}

fn short(commit: &str) -> &str {
    &commit[..commit.len().min(7)]
}

fn print_check(check: &Check, details: bool) {
    let installed = super::installed_commit();
    let here = format!(
        "{}{}",
        env!("CARGO_PKG_VERSION"),
        installed
            .as_deref()
            .map(|c| format!(" ({})", short(c)))
            .unwrap_or_default()
    );
    match check.status.as_str() {
        "available" => {
            let Some(t) = &check.target else { return };
            let changes = match check.behind {
                1 => "1 change".to_string(),
                n => format!("{n} changes"),
            };
            println!(
                "A newer harbard is available: {here} → {} ({}), {changes} on {}.",
                t.version,
                short(&t.commit),
                channel_line(check.channel)
            );
        }
        "current" => println!(
            "harbard is up to date: {here}, {}.",
            channel_line(check.channel)
        ),
        _ => {
            let e = check.error.as_ref();
            println!(
                "Could not check for updates: {} ({}).",
                e.map_or("unknown error", |e| e.message.as_str()),
                e.map_or("error", |e| e.code.as_str())
            );
        }
    }
    if !details || !check.available() {
        return;
    }
    if let Some(notes) = &check.notes {
        println!("\nWhat's new\n");
        for line in notes.lines() {
            println!("  {line}");
        }
    }
    if !check.changes.is_empty() {
        println!("\nCommits\n");
        print_changes(&check.changes, check.changes_truncated, check.behind);
    }
}

fn print_changes(changes: &[Change], truncated: bool, behind: u32) {
    const SHOWN: usize = 25;
    for c in changes.iter().take(SHOWN) {
        let label = match &c.scope {
            Some(scope) => format!("{}({scope})", c.kind),
            None => c.kind.clone(),
        };
        println!("  {}  {label:<18} {}", short(&c.commit), c.subject);
    }
    let listed = changes.len().min(SHOWN);
    let more = if truncated {
        (behind as usize).saturating_sub(listed)
    } else {
        changes.len() - listed
    };
    if more > 0 {
        println!("  … and {more} more");
    }
}

fn step_label(id: &str) -> &'static str {
    match id {
        "fetch" => "Fetch the new version",
        "cli" => "Build and install the CLI",
        "hooks" => "Connect the harnesses",
        "serve" => "Restart the bus",
        "console" => "Rebuild the console and the app",
        "desktop" => "Reopen the app",
        _ => "Step",
    }
}

fn skipped_reason(detail: Option<&str>) -> &'static str {
    match detail {
        Some("not_installed") => "not installed on this machine",
        Some("not_open") => "the app was not open",
        _ => "skipped",
    }
}

/// Print each step as it changes until the run ends.
fn follow(store: &Store) -> i32 {
    println!("\nUpdating harbard (Ctrl-C stops watching; the update keeps going).\n");
    let mut printed: Vec<(String, String)> = Vec::new();
    let started = Instant::now();
    loop {
        let Some(run) = run::view(store).and_then(|v| serde_json::from_value::<Run>(v).ok()) else {
            std::thread::sleep(Duration::from_millis(400));
            if started.elapsed() > Duration::from_secs(30) {
                eprintln!(
                    "harbard update: the run never started; see {}",
                    run::log_path(store).display()
                );
                return 1;
            }
            continue;
        };
        for s in &run.steps {
            let key = (s.id.clone(), s.state.clone());
            if printed.contains(&key) || s.state == "pending" {
                continue;
            }
            printed.push(key);
            let took = match (s.started_ms, s.finished_ms) {
                (Some(a), Some(b)) => format!(" ({:.1} s)", (b.saturating_sub(a)) as f64 / 1000.0),
                _ => String::new(),
            };
            match s.state.as_str() {
                "running" => println!("  …  {}", step_label(&s.id)),
                "done" => println!("  ✓  {}{took}", step_label(&s.id)),
                "skipped" => println!(
                    "  –  {} — {}",
                    step_label(&s.id),
                    skipped_reason(s.detail.as_deref())
                ),
                _ => println!(
                    "  ✗  {} — {}",
                    step_label(&s.id),
                    s.detail.as_deref().unwrap_or("failed")
                ),
            }
        }
        if run.finished() {
            println!();
            if run.state == "done" {
                println!(
                    "harbard is up to date{}.",
                    run.to
                        .as_deref()
                        .map(|c| format!(" at {}", short(c)))
                        .unwrap_or_default()
                );
                return 0;
            }
            let e = run.error.as_ref();
            println!(
                "The update failed: {}",
                e.map_or("unknown error", |e| e.message.as_str())
            );
            println!("Log: {}", run::log_path(store).display());
            return 1;
        }
        std::thread::sleep(Duration::from_millis(400));
    }
}
