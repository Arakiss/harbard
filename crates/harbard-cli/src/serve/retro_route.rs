//! `GET /api/v1/retro` and `GET /api/v1/retro/count`: the session retro
//! ([`harbard_core::retro`]).
//!
//! Pull-only and cold, like History. A week of transcripts is a long read,
//! so it never runs under the shell's projector (`team_route`) and never on
//! a request the console waits for to paint: a projector of its own reads
//! the week, the result answers for [`FRESH_MS`], and a stale answer is
//! served while a background pass replaces it (harbard-final-review AC1).
//! `/retro/count` reads only that cache: Activity's weekly card costs a
//! rules-file read.
//!
//! Without `?scope=team&lead=<session>` the retro covers the last week; the
//! answer also lists the teams that finished (every agent done or stopped),
//! so the Retro view can offer each one's own retro. What the operator decided is
//! read on every answer, so an approved rule leaves the count at once.

use super::workspace_route;
use harbard_core::feedback::{self, TargetKind};
use harbard_core::retro::{self, Retro, RetroInput, Scope, WEEK_MS};
use harbard_core::team::{TeamProjector, TeamRoots, TeamView};
use harbard_core::workspace::WorkspaceView;
use harbard_core::{Event, Store, Window, clock};
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

/// Standing taps read per kind for a retro.
const TAPS_PER_KIND: usize = 10_000;

/// How long one computed retro answers before a background pass replaces it.
pub const FRESH_MS: u64 = 3_600_000;

/// Team retros kept besides the week's.
const TEAMS_KEPT: usize = 16;

/// The background refresher's first pass, after serve starts.
const WARM_AFTER_SECS: u64 = 60;

/// One retro, computed once and served until it is [`FRESH_MS`] old.
#[derive(Debug)]
pub struct Computed {
    root: PathBuf,
    pub at_ms: u64,
    pub retro: Retro,
    pub finished_teams: Vec<String>,
    /// The leads the week's projection holds: a team retro needs one.
    pub leads: Vec<String>,
}

/// What the cache holds for a key.
#[derive(Debug)]
pub enum Lookup {
    Fresh(Arc<Computed>),
    Stale(Arc<Computed>),
    Missing,
}

/// Computed retros by scope, and which scopes a pass is computing now.
#[derive(Debug, Default)]
pub struct Cache {
    slots: Mutex<HashMap<String, Arc<Computed>>>,
    computing: Mutex<HashSet<String>>,
}

impl Cache {
    pub fn lookup(&self, root: &Path, key: &str, now_ms: u64) -> Lookup {
        let slots = self.slots.lock().unwrap_or_else(|e| e.into_inner());
        match slots.get(key) {
            Some(c) if c.root != root => Lookup::Missing,
            Some(c) if now_ms.saturating_sub(c.at_ms) < FRESH_MS => Lookup::Fresh(Arc::clone(c)),
            Some(c) => Lookup::Stale(Arc::clone(c)),
            None => Lookup::Missing,
        }
    }

    fn put(&self, key: &str, c: Arc<Computed>) {
        let mut slots = self.slots.lock().unwrap_or_else(|e| e.into_inner());
        if slots.len() > TEAMS_KEPT && !slots.contains_key(key) {
            // The oldest team retro goes; the week stays.
            if let Some(oldest) = slots
                .iter()
                .filter(|(k, _)| k.as_str() != WEEK_KEY)
                .min_by_key(|(_, c)| c.at_ms)
                .map(|(k, _)| k.clone())
            {
                slots.remove(&oldest);
            }
        }
        slots.insert(key.to_string(), c);
    }

    /// May this caller start a background pass for `key`? Claims it if so.
    fn claim(&self, key: &str) -> bool {
        self.computing
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(key.to_string())
    }

    fn release(&self, key: &str) {
        self.computing
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(key);
    }

    pub fn is_computing(&self, key: &str) -> bool {
        self.computing
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .contains(key)
    }
}

pub const WEEK_KEY: &str = "week";

fn key_of(scope: &Scope) -> String {
    match scope {
        Scope::Week => WEEK_KEY.to_string(),
        Scope::Team(lead) => format!("team:{lead}"),
    }
}

/// The retro's own projector, over a week: the shell's keeps a day, and a
/// pass forgets the files outside its window, so sharing it would read the
/// week again on every open and hold the shell's reads meanwhile.
fn projector() -> &'static Mutex<TeamProjector> {
    static PROJECTOR: OnceLock<Mutex<TeamProjector>> = OnceLock::new();
    PROJECTOR.get_or_init(|| Mutex::new(TeamProjector::new(TeamRoots::from_env())))
}

fn cache() -> &'static Cache {
    static CACHE: OnceLock<Cache> = OnceLock::new();
    CACHE.get_or_init(Cache::default)
}

/// Where a retro is computed: its projector and its cache. Serve uses the
/// process-wide pair; tests pass their own.
#[derive(Clone, Copy)]
pub struct Engine<'a> {
    pub projector: &'a Mutex<TeamProjector>,
    pub cache: &'a Cache,
    pub stale_secs: u64,
}

impl Engine<'static> {
    pub fn serve(stale_secs: u64) -> Self {
        Engine {
            projector: projector(),
            cache: cache(),
            stale_secs,
        }
    }
}

impl Engine<'static> {
    /// One pass, under this engine's projector and no lock of the shell's:
    /// the week's projection, the workspace read over it, the retro. Two
    /// passes for one scope do not both read: the second finds the first's.
    pub fn compute(&self, store: &Store, scope: &Scope) -> Arc<Computed> {
        let key = key_of(scope);
        let mut projector = self.projector.lock().unwrap_or_else(|e| e.into_inner());
        let now = clock::now_ms();
        if let Lookup::Fresh(c) = self.cache.lookup(store.root(), &key, now) {
            return c;
        }
        let team = projector.project(now, WEEK_MS);
        let all = store.read(Window::All).events;
        let view = workspace_route::view(store, &all, self.stale_secs, &team, now);
        let c = Arc::new(Computed {
            root: store.root().to_path_buf(),
            at_ms: now,
            retro: compute_retro(store, &all, &team, Some(&view), now, scope.clone()),
            finished_teams: retro::finished_teams(&team),
            leads: team.leads.iter().map(|l| l.session.clone()).collect(),
        });
        self.cache.put(&key, Arc::clone(&c));
        c
    }

    /// A pass on a thread of its own, unless one for `scope` is running.
    pub fn refresh_in_background(&self, store: &Store, scope: &Scope) {
        let key = key_of(scope);
        if !self.cache.claim(&key) {
            return;
        }
        let engine = *self;
        let store = store.clone();
        let scope = scope.clone();
        std::thread::spawn(move || {
            engine.compute(&store, &scope);
            engine.cache.release(&key);
        });
    }

    /// The retro for `scope`: the cached one, stale or not, with a background
    /// pass behind a stale one. A missing one is computed here when `wait`;
    /// otherwise a background pass starts and there is no answer yet.
    pub fn get(&self, store: &Store, scope: &Scope, wait: bool) -> Option<Arc<Computed>> {
        match self
            .cache
            .lookup(store.root(), &key_of(scope), clock::now_ms())
        {
            Lookup::Fresh(c) => Some(c),
            Lookup::Stale(c) => {
                self.refresh_in_background(store, scope);
                Some(c)
            }
            Lookup::Missing if wait => Some(self.compute(store, scope)),
            Lookup::Missing => {
                self.refresh_in_background(store, scope);
                None
            }
        }
    }
}

/// `scope` and `lead` from the query: a team retro needs a lead the week's
/// projection holds; anything else is the week.
pub fn scope_of(scope: Option<&str>, lead: Option<&str>, leads: &[String]) -> Scope {
    match (scope, lead) {
        (Some("team"), Some(lead)) if leads.iter().any(|l| l == lead) => {
            Scope::Team(lead.to_string())
        }
        _ => Scope::Week,
    }
}

/// `GET /api/v1/retro`: waits only for a retro never computed, and then on
/// the retro's projector, not the shell's.
pub fn retro_route(
    store: &Store,
    engine: &Engine<'static>,
    scope: Option<&str>,
    lead: Option<&str>,
) -> Value {
    let Some(week) = engine.get(store, &Scope::Week, true) else {
        return json!({ "error": "the retro is not ready" });
    };
    let scope = scope_of(scope, lead, &week.leads);
    let c = match scope {
        Scope::Week => Some(week),
        Scope::Team(_) => engine.get(store, &scope, true),
    };
    match c {
        Some(c) => retro_body(store, &c),
        None => json!({ "error": "the retro is not ready" }),
    }
}

/// `GET /api/v1/retro/count`: the rules the week proposes and the operator has not
/// decided, from the cache only. `candidates` is null until the first pass
/// lands; that pass starts here when none ran.
pub fn count_body(store: &Store, engine: &Engine<'static>) -> Value {
    match engine.get(store, &Scope::Week, false) {
        Some(c) => {
            let book = harbard_core::rules::load(store);
            json!({
                "v": 1,
                "candidates": retro::candidates(&c.retro, &book.decided).len(),
                "computed_ms": c.at_ms,
                "computing": engine.cache.is_computing(WEEK_KEY),
            })
        }
        None => json!({ "v": 1, "candidates": null, "computed_ms": null, "computing": true }),
    }
}

/// The week's retro kept warm: a first pass a minute after serve starts,
/// then one whenever the cached one is older than [`FRESH_MS`]. Each pass
/// after the first reads only what the transcripts appended.
pub fn start_hourly(store: Store, stale_secs: u64, shutdown: Arc<AtomicBool>) {
    std::thread::spawn(move || {
        let engine = Engine::serve(stale_secs);
        let mut ticks: u64 = 0;
        while !shutdown.load(Ordering::Relaxed) {
            std::thread::sleep(std::time::Duration::from_secs(1));
            ticks += 1;
            if ticks < WARM_AFTER_SECS || !ticks.is_multiple_of(60) {
                continue;
            }
            let now = clock::now_ms();
            if !matches!(
                engine.cache.lookup(store.root(), WEEK_KEY, now),
                Lookup::Fresh(_)
            ) && engine.cache.claim(WEEK_KEY)
            {
                engine.compute(&store, &Scope::Week);
                engine.cache.release(WEEK_KEY);
            }
        }
    });
}

fn compute_retro(
    store: &Store,
    events: &[Event],
    team: &TeamView,
    view: Option<&WorkspaceView>,
    now: u64,
    scope: Scope,
) -> Retro {
    let taps: Vec<feedback::Record> = TargetKind::ALL
        .into_iter()
        .flat_map(|k| feedback::active_of_kind(store, k, TAPS_PER_KIND).unwrap_or_default())
        .collect();
    let since = match &scope {
        Scope::Week => now.saturating_sub(WEEK_MS),
        Scope::Team(lead) => team
            .leads
            .iter()
            .find(|l| &l.session == lead)
            .and_then(|l| l.agents.iter().filter_map(|a| a.first_ms).min())
            .unwrap_or(now.saturating_sub(WEEK_MS)),
    };
    retro::compute(&RetroInput {
        now_ms: now,
        since_ms: since,
        scope,
        events,
        team,
        view,
        feedback: &taps,
        // Name 👍 / 👎 and renames are the workshop's votes: one source.
        name_votes: &harbard_core::workshop::Registry::load(store.root()).votes,
    })
}

/// The lessons the Retro proposes at this moment, as `GET /retro` shows
/// them (the cached retro, less what the operator decided): the only keys
/// `rule-approve` accepts. The week's, and the team's when
/// `lead` names one the week holds.
pub fn candidates_now(
    store: &Store,
    engine: &Engine<'static>,
    lead: Option<&str>,
) -> Vec<retro::Lesson> {
    let Some(week) = engine.get(store, &Scope::Week, true) else {
        return Vec::new();
    };
    let book = harbard_core::rules::load(store);
    let mut out = retro::candidates(&week.retro, &book.decided);
    if let Scope::Team(lead) = scope_of(Some("team"), lead, &week.leads)
        && let Some(team) = engine.get(store, &Scope::Team(lead), true)
    {
        for l in retro::candidates(&team.retro, &book.decided) {
            if !out.iter().any(|o| o.key == l.key) {
                out.push(l);
            }
        }
    }
    out
}

/// The answer for one computed retro. The rule book is read now, so what
/// the operator decided a moment ago is already out of the candidates.
pub fn retro_body(store: &Store, c: &Computed) -> Value {
    let book = harbard_core::rules::load(store);
    let candidates = retro::candidates(&c.retro, &book.decided);
    json!({
        "v": 1,
        "computed_ms": c.at_ms,
        "retro": c.retro,
        "candidates": candidates,
        "rules": book.active,
        "retired": book.retired,
        "finished_teams": c.finished_teams,
        // "Was it you?": open, and answered in the last days.
        "checks": harbard_core::workspace::DeskState::load(store).recent_checks(clock::now_ms()),
        // Only whether a sink is configured: nothing is run to find out.
        "memory_sink": {
            "configured": crate::memory_sink::command(store).is_some(),
            "allowed": super::feedback_route::memory_sink_allowed(store),
        },
    })
}

/// An engine over an empty projects root under `dir`, for other routes'
/// tests: it never reads the real transcripts.
#[cfg(test)]
pub(crate) fn test_engine(dir: &Path) -> Engine<'static> {
    let roots = TeamRoots {
        home: dir.join("os-home"),
        claude_projects: dir.join("os-home/.claude/projects"),
    };
    Engine {
        projector: Box::leak(Box::new(Mutex::new(TeamProjector::new(roots)))),
        cache: Box::leak(Box::new(Cache::default())),
        stale_secs: 300,
    }
}

/// An empty computed retro for `root`, for other routes' tests.
#[cfg(test)]
pub(crate) fn empty_computed(root: &Path, at_ms: u64) -> Arc<Computed> {
    Arc::new(Computed {
        root: root.to_path_buf(),
        at_ms,
        retro: Retro::default(),
        finished_teams: Vec::new(),
        leads: Vec::new(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "harbard-retro-route-{tag}-{}-{}",
            std::process::id(),
            clock::now_ms()
        ));
        std::fs::create_dir_all(dir.join("home/.claude/projects")).unwrap();
        dir
    }

    /// An engine over an empty projects root of its own.
    fn engine(dir: &Path) -> Engine<'static> {
        let roots = TeamRoots {
            home: dir.join("home"),
            claude_projects: dir.join("home/.claude/projects"),
        };
        Engine {
            projector: Box::leak(Box::new(Mutex::new(TeamProjector::new(roots)))),
            cache: Box::leak(Box::new(Cache::default())),
            stale_secs: 300,
        }
    }

    fn computed(root: &Path, at_ms: u64) -> Arc<Computed> {
        Arc::new(Computed {
            root: root.to_path_buf(),
            at_ms,
            retro: Retro::default(),
            finished_teams: Vec::new(),
            leads: vec!["lead-1".into()],
        })
    }

    #[test]
    fn a_team_retro_needs_a_lead_the_projection_holds() {
        let leads = vec!["lead-1".to_string()];
        assert_eq!(scope_of(Some("team"), Some("ghost"), &leads), Scope::Week);
        assert_eq!(scope_of(None, None, &leads), Scope::Week);
        assert_eq!(
            scope_of(Some("team"), Some("lead-1"), &leads),
            Scope::Team("lead-1".into())
        );
    }

    #[test]
    fn the_week_answers_with_seven_days_and_no_finished_team() {
        let dir = temp("week");
        let store = Store::at(dir.join("store"));
        let body = retro_route(&store, &engine(&dir), None, None);
        assert_eq!(body["retro"]["scope"], "week");
        assert_eq!(body["retro"]["daily"].as_array().unwrap().len(), 7);
        assert!(body["finished_teams"].as_array().unwrap().is_empty());
        assert!(body["computed_ms"].as_u64().is_some());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// AC1: the week's pass ran under the shell's projector, so every
    /// `/workspace` read and every write waited for it. It must finish while
    /// the shell's projector is held by someone else.
    #[test]
    fn a_retro_pass_never_waits_for_the_shell_projector() {
        let dir = temp("lock");
        let store = Store::at(dir.join("store"));
        let engine = engine(&dir);
        let held = super::super::team_route::hold_projector_for_test();
        let (tx, rx) = std::sync::mpsc::channel();
        let s = store.clone();
        std::thread::spawn(move || {
            engine.compute(&s, &Scope::Week);
            let _ = tx.send(());
        });
        let done = rx.recv_timeout(std::time::Duration::from_secs(20));
        drop(held);
        assert!(done.is_ok(), "the retro waited for the shell's projector");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_hour_old_retro_is_served_and_replaced_in_the_background() {
        let dir = temp("stale");
        let store = Store::at(dir.join("store"));
        let engine = engine(&dir);
        let now = clock::now_ms();
        engine
            .cache
            .put(WEEK_KEY, computed(store.root(), now - FRESH_MS - 1));
        let served = engine.get(&store, &Scope::Week, false).unwrap();
        assert_eq!(served.at_ms, now - FRESH_MS - 1, "the stale one answers");
        // The background pass lands a fresh one.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
        loop {
            if let Lookup::Fresh(c) = engine.cache.lookup(store.root(), WEEK_KEY, clock::now_ms()) {
                assert!(c.at_ms >= now);
                break;
            }
            assert!(std::time::Instant::now() < deadline, "no background pass");
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_fresh_retro_is_not_computed_again() {
        let dir = temp("fresh");
        let store = Store::at(dir.join("store"));
        let engine = engine(&dir);
        let at = clock::now_ms() - 1_000;
        engine.cache.put(WEEK_KEY, computed(store.root(), at));
        // Held projector: a pass would block, so an answer proves none ran.
        let _held = engine.projector.lock().unwrap();
        assert_eq!(engine.get(&store, &Scope::Week, true).unwrap().at_ms, at);
        assert!(!engine.cache.is_computing(WEEK_KEY));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The weekly card's read: from the cache only, never a pass on the
    /// request, and what the operator decided leaves the count at once.
    #[test]
    fn the_count_reads_the_cache_and_the_rule_book_of_now() {
        let dir = temp("count");
        let store = Store::at(dir.join("store"));
        let engine = engine(&dir);
        {
            let _held = engine.projector.lock().unwrap();
            let out = count_body(&store, &engine);
            assert!(out["candidates"].is_null(), "nothing computed yet: {out}");
            assert_eq!(out["computing"], true);
        }
        let mut c = Computed {
            root: store.root().to_path_buf(),
            at_ms: clock::now_ms(),
            retro: Retro::default(),
            finished_teams: Vec::new(),
            leads: Vec::new(),
        };
        c.retro.lessons = vec![retro::Lesson {
            key: "secret_leak",
            title: "Secrets",
            evidence: String::new(),
            evidence_code: "",
            evidence_params: Default::default(),
            rule: String::new(),
            count: 1,
            dates: Vec::new(),
        }];
        engine.cache.put(WEEK_KEY, Arc::new(c));
        assert_eq!(count_body(&store, &engine)["candidates"], 1);
        harbard_core::rules::approve(
            &store,
            crate::serve::operator_note_event(
                "",
                None,
                harbard_core::NoteKind::Fyi,
                None,
                clock::now_ms(),
            ),
            "secret_leak",
            "Secrets",
            "Never paste a secret.",
            false,
            false,
        )
        .unwrap();
        assert_eq!(count_body(&store, &engine)["candidates"], 0);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
