//! Ported from the console's `lib/workspace.ts` (the Home selections),
//! `lib/hidden.ts`, `lib/legible.ts` (`channelHeadline`, `channelPlaces`),
//! `lib/serve-text.ts` (`roleVars`, `localRole`), `lib/current-workspace.ts`
//! and `lib/deliveries.ts` (`pendingCount`).
//!
//! Selections borrow from the view and return plain structs. Wording takes a
//! [`Words`] (an `&App` or a view's `&mut Context`) so it can be tested with
//! the real dictionaries and no window.

use super::members::{channel_agents, channel_members};
use super::rooms;
use crate::model::{Agent, Channel, Message, Params, WorkspaceTile, WorkspaceView, WriterWarning};
use crate::nav::Route;
use gpui::{App, Context, SharedString};
use std::borrow::Cow;
use std::cmp::Ordering;
use std::collections::HashMap;

const MIN: u64 = 60_000;
const HOUR: u64 = 60 * MIN;

// ---------------------------------------------------------------------------
// Words: the dictionaries, behind a trait so tests need no App
// ---------------------------------------------------------------------------

/// Where wording comes from: the app's dictionaries (`&App`, a view's
/// `&mut Context`), or a test's copy of them.
pub trait Words {
    /// `ns.key` with its `{holes}` filled.
    fn text(&self, key: &str, vars: &[(&str, &str)]) -> SharedString;
    /// serve's stable code worded in this language, else serve's text.
    fn serve(&self, code: Option<&str>, params: Option<&Params>, fallback: &str) -> SharedString;
    /// "today at 22:41", "on 5 Nov at 22:41".
    fn at_date(&self, ms: u64, now: u64) -> SharedString;
}

impl Words for App {
    fn text(&self, key: &str, vars: &[(&str, &str)]) -> SharedString {
        crate::i18n::tv(self, key, vars)
    }
    fn serve(&self, code: Option<&str>, params: Option<&Params>, fallback: &str) -> SharedString {
        crate::i18n::serve_text(self, code, params, fallback)
    }
    fn at_date(&self, ms: u64, now: u64) -> SharedString {
        crate::ui::format::at_date(ms, now, self)
    }
}

impl<T> Words for Context<'_, T> {
    fn text(&self, key: &str, vars: &[(&str, &str)]) -> SharedString {
        let app: &App = self;
        app.text(key, vars)
    }
    fn serve(&self, code: Option<&str>, params: Option<&Params>, fallback: &str) -> SharedString {
        let app: &App = self;
        app.serve(code, params, fallback)
    }
    fn at_date(&self, ms: u64, now: u64) -> SharedString {
        let app: &App = self;
        app.at_date(ms, now)
    }
}

/// A plural by count: `baseOne` / `baseMany`, with `count` filled.
fn plural(w: &impl Words, base: &str, count: usize, vars: &[(&str, &str)]) -> SharedString {
    let key = if count == 1 { format!("{base}One") } else { format!("{base}Many") };
    let n = count.to_string();
    let mut all = vars.to_vec();
    all.push(("count", &n));
    w.text(&key, &all)
}

// ---------------------------------------------------------------------------
// Sorting and small text helpers
// ---------------------------------------------------------------------------

/// The console sorts names with `localeCompare(…, "es")`. This approximates
/// it: case and accents are ignored, ñ sorts after n, and a tie falls back to
/// the raw strings.
pub fn collate(a: &str, b: &str) -> Ordering {
    fold(a).cmp(&fold(b)).then_with(|| a.cmp(b))
}

fn fold(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars().flat_map(char::to_lowercase) {
        match c {
            'á' | 'à' | 'ä' | 'â' | 'ã' => out.push('a'),
            'é' | 'è' | 'ë' | 'ê' => out.push('e'),
            'í' | 'ì' | 'ï' | 'î' => out.push('i'),
            'ó' | 'ò' | 'ö' | 'ô' | 'õ' => out.push('o'),
            'ú' | 'ù' | 'ü' | 'û' => out.push('u'),
            'ç' => out.push('c'),
            'ñ' => out.push_str("n\u{7f}"),
            c => out.push(c),
        }
    }
    out
}

/// The last path segment (`fileName`).
pub fn file_name(path: &str) -> &str {
    let trimmed = path.trim_end_matches('/');
    match trimmed.rsplit('/').next() {
        Some(last) if !last.is_empty() => last,
        _ => path,
    }
}

/// `/Users/<x>/...` → `~/...` (`shortPath`), whoever's home it is.
pub fn short_path(path: &str) -> String {
    if let Some(rest) = path.strip_prefix("/Users/")
        && !rest.is_empty()
    {
        let cut = rest.find('/').unwrap_or(rest.len());
        if cut > 0 {
            return format!("~{}", &rest[cut..]);
        }
    }
    path.to_string()
}

/// "a, b and c" (`workspace.listJoin`).
pub fn list_join(names: &[SharedString], w: &impl Words) -> SharedString {
    match names {
        [] => SharedString::default(),
        [one] => one.clone(),
        [head @ .., last] => {
            let head = head.iter().map(|s| s.as_ref()).collect::<Vec<&str>>().join(", ");
            w.text("workspace.listJoin", &[("head", &head), ("last", last)])
        }
    }
}

// ---------------------------------------------------------------------------
// Names and roles (serve-text.ts)
// ---------------------------------------------------------------------------

/// Each role by its key: the masculine word, then the feminine one.
const ROLE_WORD: &[(&str, &str, &str)] = &[
    ("builder", "roleBuilder", "roleBuilderF"),
    ("reviewer", "roleReviewer", "roleReviewerF"),
    ("integration", "roleIntegration", "roleIntegration"),
    ("lead", "roleLead", "roleLeadF"),
    ("research", "roleResearch", "roleResearch"),
    ("study", "roleStudy", "roleStudy"),
    ("tests", "roleTests", "roleTests"),
    ("fix", "roleFix", "roleFix"),
    ("docs", "roleDocs", "roleDocs"),
    ("design", "roleDesign", "roleDesign"),
    ("audit", "roleAudit", "roleAudit"),
    ("ops", "roleOps", "roleOps"),
    ("eval", "roleEval", "roleEval"),
    ("migration", "roleMigration", "roleMigration"),
    ("upgrade", "roleUpgrade", "roleUpgrade"),
    ("holdout", "roleHoldout", "roleHoldout"),
    ("session", "roleSession", "roleSession"),
];

fn role_word(role: &str, feminine: bool, w: &impl Words) -> String {
    match ROLE_WORD.iter().find(|(k, _, _)| *k == role) {
        Some((_, m, f)) => w.text(&format!("serve.{}", if feminine { f } else { m }), &[]).to_string(),
        None => role.to_string(),
    }
}

fn param_text(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

/// A coded role's params, its `role` replaced by the word to show, with its
/// number when it has one (`roleVars`).
pub fn role_vars(params: &Params, w: &impl Words) -> Params {
    let role = params.get("role").map(param_text).unwrap_or_default();
    let feminine = params.get("gender").and_then(|g| g.as_str()) == Some("f");
    let word = role_word(&role, feminine, w);
    let shown = match params.get("number") {
        None | Some(serde_json::Value::Null) => word,
        Some(n) => format!("{word} {}", param_text(n)),
    };
    let mut out = params.clone();
    out.insert("role".into(), serde_json::Value::String(shown));
    out
}

/// A role as serve words it, `reviewer 2`, in this language (`localRole`).
pub fn local_role(role: &str, w: &impl Words) -> String {
    let (word, number) = match role.split_once(' ') {
        Some((word, n)) if !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()) => (word, Some(n)),
        Some(_) => return role.to_string(),
        None => (role, None),
    };
    if word.is_empty() || word.chars().any(char::is_whitespace) || !ROLE_WORD.iter().any(|(k, _, _)| *k == word) {
        return role.to_string();
    }
    match number {
        Some(n) => format!("{} {n}", role_word(word, false, w)),
        None => role_word(word, false, w),
    }
}

/// `shownName`: serve's fixed names (the unnamed lead, a bare session).
pub fn shown_name_with(name: &str, w: &impl Words) -> SharedString {
    if name == super::LEAD_AUTHOR {
        return w.text("workspace.leadAuthor", &[]);
    }
    if let Some(id) = name.strip_prefix("session ")
        && !id.is_empty()
        && id.chars().count() <= 8
        && !id.chars().any(char::is_whitespace)
    {
        let mut p = Params::new();
        p.insert("id".into(), serde_json::Value::String(id.into()));
        return w.serve(Some("name.session.id"), Some(&p), name);
    }
    name.to_string().into()
}

/// `agentName`: a `name.team_role` name has its role worded first.
/// [`super::agent_name`] calls this.
pub fn agent_name_with(a: &Agent, w: &impl Words) -> SharedString {
    let fallback = shown_name_with(&a.name, w);
    let worded;
    let params = match (&a.name_params, a.name_code.as_deref()) {
        (Some(p), Some("name.team_role")) => {
            worded = role_vars(p, w);
            Some(&worded)
        }
        (p, _) => p.as_ref(),
    };
    w.serve(a.name_code.as_deref(), params, &fallback)
}

/// `agentLabel`: the name an agent key shows; `lead:<session>` is a lead
/// session itself.
pub fn agent_label_with(view: &WorkspaceView, key: &str, w: &impl Words) -> SharedString {
    if let Some(id) = key.strip_prefix("lead:") {
        let short: String = id.chars().take(8).collect();
        return w.text("workspace.leadSessionNamed", &[("id", &short)]);
    }
    match view.agent(key) {
        Some(a) => agent_name_with(a, w),
        None => shown_name_with(&key.chars().take(8).collect::<String>(), w),
    }
}

/// The line under a workshop name (`agentRoleLine`): a coded role worded in
/// this language, else serve's line, else empty.
pub fn agent_role_line(a: &Agent, w: &impl Words) -> SharedString {
    let fallback = a.role_line.as_deref().unwrap_or("");
    let vars = a.role_line_params.as_ref().map(|p| role_vars(p, w));
    w.serve(a.role_line_code.as_deref(), vars.as_ref(), fallback)
}

// ---------------------------------------------------------------------------
// The sidebar's agents
// ---------------------------------------------------------------------------

/// Agents with a record this recent appear in the sidebar.
const SIDEBAR_ACTIVE_MS: u64 = 24 * HOUR;
/// Rows shown before "See all".
pub const SIDEBAR_ROWS: usize = 8;
/// The filter box shows from this many listed agents.
pub const FILTER_FROM: usize = 8;
/// Idle, finished or without data this long: under "Inactive".
const INACTIVE_MS: u64 = 30 * MIN;
/// A closed agent (its lead stopped it) moves there sooner.
const CLOSED_INACTIVE_MS: u64 = 10 * MIN;
/// Faces that are work in progress: never inactive.
const BUSY: &[&str] = &[
    "thinking", "running", "testing", "editing", "reading", "web", "delegating", "waiting", "blocked", "asleep",
];

#[derive(Debug, Default, Clone)]
pub struct SidebarAgents<'a> {
    /// "Main sessions": open sessions, busy or not.
    pub sessions: Vec<&'a Agent>,
    /// The first eight active agents.
    pub shown: Vec<&'a Agent>,
    /// The other active agents, behind "See all".
    pub rest: Vec<&'a Agent>,
    /// Behind the folded "Inactive (n)".
    pub inactive: Vec<&'a Agent>,
}

impl SidebarAgents<'_> {
    /// Everything listed: decides whether the filter box shows.
    pub fn listed(&self) -> usize {
        self.sessions.len() + self.shown.len() + self.rest.len() + self.inactive.len()
    }
}

/// A main session still running: the one to talk to.
fn is_open_session(a: &Agent) -> bool {
    a.kind == "session" && a.state != "done" && !a.lost
}

/// The sidebar's agents (`sidebarAgents`): active in the last 24 h, newest
/// first, none temporary or hidden; open sessions apart; then active ones
/// (eight shown, the rest behind "See all") and inactive ones.
pub fn sidebar_agents(view: &WorkspaceView, now: u64) -> SidebarAgents<'_> {
    let since = now.saturating_sub(SIDEBAR_ACTIVE_MS);
    let mut listed: Vec<&Agent> = view
        .agents
        .iter()
        .filter(|a| !a.temporary && !a.hidden && a.last_action_ms.unwrap_or(0) >= since)
        .collect();
    listed.sort_by(|a, b| {
        b.last_action_ms
            .unwrap_or(0)
            .cmp(&a.last_action_ms.unwrap_or(0))
            .then_with(|| collate(&a.key, &b.key))
    });
    let mut out = SidebarAgents::default();
    let mut active = Vec::new();
    for a in listed {
        if is_open_session(a) {
            out.sessions.push(a);
        } else if is_inactive(a, now) {
            out.inactive.push(a);
        } else {
            active.push(a);
        }
    }
    let rest = active.split_off(active.len().min(SIDEBAR_ROWS));
    out.shown = active;
    out.rest = rest;
    out
}

/// Whether an agent goes into "Inactive" (`isInactive`): closed for ten
/// minutes, or idle, finished or without data for thirty. One that is
/// working, waiting or blocked never does.
pub fn is_inactive(a: &Agent, now: u64) -> bool {
    let state = if a.activity.state.is_empty() { "normal" } else { a.activity.state.as_str() };
    if BUSY.contains(&state) && !a.lost {
        return false;
    }
    let quiet = now.saturating_sub(a.last_action_ms.unwrap_or(0));
    if a.closed || a.activity.detail.as_deref() == Some("closed") {
        return quiet > CLOSED_INACTIVE_MS;
    }
    quiet > INACTIVE_MS
}

/// The sidebar's agent filter (`filterAgents`): by name, handle or team,
/// case-insensitive. An empty query keeps everything.
pub fn filter_agents<'a>(agents: &[&'a Agent], query: &str) -> Vec<&'a Agent> {
    let q = query.trim().to_lowercase();
    if q.is_empty() {
        return agents.to_vec();
    }
    let has = |s: Option<&str>| s.is_some_and(|s| s.to_lowercase().contains(&q));
    agents
        .iter()
        .copied()
        .filter(|a| has(Some(&a.name)) || has(a.handle.as_deref()) || has(a.team.as_deref()))
        .collect()
}

/// "Temporary sessions": agents of throwaway sessions, none hidden.
pub fn temporary_agents(view: &WorkspaceView) -> Vec<&Agent> {
    view.agents.iter().filter(|a| a.temporary && !a.hidden).collect()
}

/// "Hidden (n)": every hidden agent, by name (Spanish order, approximated),
/// then key.
pub fn hidden_agents(view: &WorkspaceView) -> Vec<&Agent> {
    let mut out: Vec<&Agent> = view.agents.iter().filter(|a| a.hidden).collect();
    out.sort_by(|a, b| collate(&a.name, &b.name).then_with(|| collate(&a.key, &b.key)));
    out
}

/// The ⋯ menu's item for an agent: its words and the action to post with
/// `{agent: key}`.
#[derive(Debug, Clone, PartialEq)]
pub struct HideAction {
    pub label: SharedString,
    /// `hide-agent` or `show-agent`.
    pub action: &'static str,
}

pub fn hide_action(a: &Agent, w: &impl Words) -> HideAction {
    if a.hidden {
        HideAction { label: w.text("shell.show", &[]), action: "show-agent" }
    } else {
        HideAction { label: w.text("shell.hide", &[]), action: "hide-agent" }
    }
}

// ---------------------------------------------------------------------------
// Channels
// ---------------------------------------------------------------------------

#[derive(Debug, Default, Clone)]
pub struct SidebarChannels<'a> {
    /// Project channels with recent activity.
    pub shown: Vec<&'a Channel>,
    /// The quiet ones, under "More channels (n)".
    pub more: Vec<&'a Channel>,
}

/// Project channels for the sidebar (`sidebarChannels`): no rooms, nothing
/// archived; quiet ones apart.
pub fn sidebar_channels(view: &WorkspaceView) -> SidebarChannels<'_> {
    let mut out = SidebarChannels::default();
    for c in view.channels.iter().filter(|c| !c.is_room() && !c.archived) {
        if c.quiet {
            out.more.push(c);
        } else {
            out.shown.push(c);
        }
    }
    out
}

/// Where Home opens: the first project channel; a room never is.
pub fn default_channel_id(view: &WorkspaceView) -> Option<&str> {
    view.channels.iter().find(|c| !c.is_room() && !c.archived).map(|c| c.id.as_str())
}

/// A channel by id; the three built-in rooms exist whatever serve listed
/// (a placeholder when it did not).
pub fn find_channel<'a>(view: &'a WorkspaceView, id: &str) -> Option<Cow<'a, Channel>> {
    match view.channel(id) {
        Some(c) => Some(Cow::Borrowed(c)),
        None => rooms::built_in_placeholder(id).map(Cow::Owned),
    }
}

/// Where a channel row goes: `#lobby` has its own screen.
pub fn channel_route(id: &str) -> Route {
    Route::channel(id)
}

/// A channel row's tooltip: a room's purpose, else its repository, short.
pub fn channel_title(c: &Channel) -> String {
    match c.purpose.as_deref() {
        Some(p) if c.is_room() && !p.is_empty() => p.to_string(),
        _ => short_path(&c.repo),
    }
}

/// Top-level messages of a channel (`channelStream`): replies live in their
/// thread; protocol records and peer chatter stay out.
pub fn channel_stream<'a>(view: &'a WorkspaceView, channel: &str) -> Vec<&'a Message> {
    view.messages
        .iter()
        .filter(|m| m.channels.iter().any(|c| c == channel) && m.reply_to.is_none() && m.structured.is_none() && m.from != "peer")
        .collect()
}

/// Messages that have a thread, the one answered last first (`threads`).
pub fn threads(view: &WorkspaceView) -> Vec<&Message> {
    let at: HashMap<&str, u64> = view.messages.iter().map(|m| (m.id.as_str(), m.ts_ms)).collect();
    let mut out: Vec<(&Message, u64)> = view
        .messages
        .iter()
        .filter(|m| !m.replies.is_empty())
        .map(|m| {
            let last = m.replies.iter().map(|r| at.get(r.as_str()).copied().unwrap_or(0)).fold(m.ts_ms, u64::max);
            (m, last)
        })
        .collect();
    out.sort_by_key(|a| std::cmp::Reverse(a.1));
    out.into_iter().map(|(m, _)| m).collect()
}

/// Replies of a message, oldest first (`thread`). Empty when the parent is
/// unknown.
pub fn thread<'a>(view: &'a WorkspaceView, id: &str) -> Vec<&'a Message> {
    let Some(parent) = view.message(id) else {
        return Vec::new();
    };
    let mut out: Vec<&Message> = parent.replies.iter().filter_map(|r| view.message(r)).collect();
    out.sort_by_key(|m| m.ts_ms);
    out
}

/// Why a channel is in the list, and the agents the line is about.
#[derive(Debug, Clone)]
pub struct ChannelWhy<'a> {
    pub line: SharedString,
    pub agents: Vec<&'a Agent>,
}

/// `channelWhy`: who worked here in the last 24 h; else when it last had
/// activity; else that agents only read it with git.
pub fn channel_why<'a>(view: &'a WorkspaceView, channel: &Channel, w: &impl Words) -> ChannelWhy<'a> {
    let recent: Vec<&Agent> = channel.recent.iter().filter_map(|k| view.agent(k)).collect();
    if !recent.is_empty() {
        let line = if recent.len() == 1 {
            w.text("workspace.whySessionsOne", &[])
        } else {
            w.text("workspace.whySessionsMany", &[("n", &recent.len().to_string())])
        };
        return ChannelWhy { line, agents: recent };
    }
    let readers = channel_agents(view, channel);
    if let Some(ms) = channel.active_ms.filter(|ms| *ms > 0) {
        let when = w.at_date(ms, view.now_ms);
        return ChannelWhy { line: w.text("workspace.whyIdle", &[("when", &when)]), agents: readers };
    }
    let line = if readers.len() == 1 {
        w.text("workspace.whyReadOne", &[])
    } else {
        w.text("workspace.whyReadMany", &[("n", &readers.len().to_string())])
    };
    ChannelWhy { line, agents: readers }
}

/// The channel header's line: "37 sessions today · 1 working now".
#[derive(Debug, Clone)]
pub struct Headline<'a> {
    /// "37 sessions today", "1 session in the last 24 h", or the why line.
    pub sessions: SharedString,
    /// "1 working now" / "nobody working now"; None in the fallback with
    /// nobody working.
    pub now: Option<SharedString>,
    /// How many work here now (0 → `data-zero`, and "work elsewhere" shows).
    pub working: usize,
    /// The sessions the first half counts, for the list behind it.
    pub agents: Vec<&'a Agent>,
}

/// `channelHeadline`: sessions today (since local midnight), else in the
/// last 24 h, else [`channel_why`]; and who works now.
pub fn channel_headline<'a>(view: &'a WorkspaceView, channel: &Channel, w: &impl Words) -> Headline<'a> {
    let today_day = crate::ui::format::local_day(view.now_ms);
    let recent: Vec<&Agent> = channel.recent.iter().filter_map(|k| view.agent(k)).collect();
    let today: Vec<&Agent> = recent
        .iter()
        .copied()
        .filter(|a| crate::ui::format::local_day(a.last_action_ms.unwrap_or(0)) >= today_day)
        .collect();
    let working = channel_members(view, channel).now;
    let now = if working > 0 {
        w.text("activity.channelWorkingNow", &[("count", &working.to_string())])
    } else {
        w.text("activity.channelNobodyWorking", &[])
    };
    let sessions = |n: usize| plural(w, "activity.channelSessions", n, &[]);
    if !today.is_empty() {
        let s = sessions(today.len());
        return Headline {
            sessions: w.text("activity.channelHeadlineToday", &[("sessions", &s)]),
            now: Some(now),
            working,
            agents: today,
        };
    }
    if !recent.is_empty() {
        let s = sessions(recent.len());
        return Headline {
            sessions: w.text("activity.channelHeadlineRecent", &[("sessions", &s)]),
            now: Some(now),
            working,
            agents: recent,
        };
    }
    let why = channel_why(view, channel, w);
    Headline { sessions: why.line, now: (working > 0).then_some(now), working, agents: why.agents }
}

/// One copy of a repository in the header's "ℹ".
#[derive(Debug, Clone, PartialEq)]
pub struct Place {
    /// "main copy", a path under the repository, or a `~` path.
    pub path: SharedString,
    /// The full path, for the tooltip.
    pub full: String,
    pub branch: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Places {
    /// The repository, or "3 copies of ~/Projects/acme".
    pub title: SharedString,
    pub copies: Vec<Place>,
}

/// `channelPlaces`: the repository and each checkout with its branch. The
/// "ℹ" is hidden when `copies` is empty.
pub fn channel_places(channel: &Channel, w: &impl Words) -> Places {
    let root = channel.repo.trim_end_matches('/');
    let copies: Vec<Place> = channel
        .checkouts
        .iter()
        .map(|c| {
            let path: SharedString = if c.path == root {
                w.text("activity.placesMainCopy", &[])
            } else if let Some(rel) = c.path.strip_prefix(root).and_then(|r| r.strip_prefix('/')) {
                rel.to_string().into()
            } else {
                short_path(&c.path).into()
            };
            Place { path, full: c.path.clone(), branch: c.branch.clone().filter(|b| !b.is_empty()) }
        })
        .collect();
    let repo = short_path(&channel.repo);
    let title = if copies.len() <= 1 {
        repo.into()
    } else {
        w.text("activity.placesCopies", &[("count", &copies.len().to_string()), ("repo", &repo)])
    };
    Places { title, copies }
}

/// The two-writers strip's bold lead: "Two agents in the same checkout:".
pub fn warning_lead(warning: &WriterWarning, w: &impl Words) -> SharedString {
    if warning.agents.len() == 2 {
        w.text("home.twoAgentsSameCheckoutLead", &[])
    } else {
        w.text("home.agentsSameCheckoutLead", &[("count", &warning.agents.len().to_string())])
    }
}

/// The rest of the strip: "a, b writing in orchard-wt since 22:41."
pub fn warning_text(view: &WorkspaceView, warning: &WriterWarning, w: &impl Words) -> SharedString {
    let agents = warning
        .agents
        .iter()
        .map(|k| agent_label_with(view, k, w).to_string())
        .collect::<Vec<String>>()
        .join(", ");
    w.text(
        "home.agentsWritingSince",
        &[
            ("agents", &agents),
            ("file", file_name(&warning.checkout)),
            ("time", &crate::ui::format::hhmm(warning.since_ms)),
        ],
    )
}

// ---------------------------------------------------------------------------
// Workspaces, Deliveries, Lobby
// ---------------------------------------------------------------------------

/// The sidebar head's name: the current workspace's, else "harbard".
pub fn current_name(tiles: &[WorkspaceTile]) -> &str {
    tiles.iter().find(|t| t.current).map(|t| t.name.as_str()).unwrap_or("harbard")
}

/// The other workspace with the most agents working now, or None.
pub fn working_elsewhere(tiles: &[WorkspaceTile]) -> Option<&WorkspaceTile> {
    let mut best: Option<&WorkspaceTile> = None;
    for t in tiles {
        if !t.current && t.working > 0 && best.is_none_or(|b| t.working > b.working) {
            best = Some(t);
        }
    }
    best
}

/// "3 working in Acme" (the "work elsewhere" button).
pub fn working_in_text(tile: &WorkspaceTile, w: &impl Words) -> SharedString {
    w.text("home.workingIn", &[("count", &tile.working.to_string()), ("name", &tile.name)])
}

/// Deliveries not installed yet: the sidebar's always-shown count.
pub fn pending_count(view: &WorkspaceView) -> usize {
    view.shipments.iter().filter(|s| s.pending).count()
}

/// The Lobby's three counts, over every entry: here (live), around (idle,
/// stale) and gone (ended).
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct LobbyCounts {
    pub here: usize,
    pub around: usize,
    pub gone: usize,
}

pub fn lobby_counts(view: &WorkspaceView) -> LobbyCounts {
    let mut out = LobbyCounts::default();
    for e in &view.lobby {
        match e.presence.as_str() {
            "live" => out.here += 1,
            "idle" | "stale" => out.around += 1,
            "ended" => out.gone += 1,
            _ => {}
        }
    }
    out
}

/// The sidebar Lobby row's badge: who is here now (shown when > 0).
pub fn lobby_here_count(view: &WorkspaceView) -> usize {
    lobby_counts(view).here
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// The real dictionaries without an App, for tests in every derive file.
#[cfg(test)]
pub(crate) mod test_words {
    use super::Words;
    use crate::model::Params;
    use gpui::SharedString;
    use std::collections::HashMap;

    pub struct Dict(HashMap<String, String>);

    fn flatten(prefix: &str, v: &serde_json::Value, out: &mut HashMap<String, String>) {
        match v {
            serde_json::Value::Object(map) => {
                for (k, v) in map {
                    flatten(&format!("{prefix}.{k}"), v, out);
                }
            }
            serde_json::Value::String(s) => {
                out.insert(prefix.to_string(), s.clone());
            }
            _ => {}
        }
    }

    pub fn dict(locale: &str) -> Dict {
        let mut out = HashMap::new();
        let prefix = format!("locales/{locale}/");
        for path in crate::assets::Assets::local_paths(&prefix) {
            let ns = path.strip_prefix(&prefix).and_then(|f| f.strip_suffix(".json")).unwrap().to_string();
            let bytes = crate::assets::Assets::local(&path).unwrap();
            let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
            flatten(&ns, &v, &mut out);
        }
        Dict(out)
    }

    pub fn en() -> Dict {
        dict("en")
    }

    pub fn es() -> Dict {
        dict("es")
    }

    impl Words for Dict {
        fn text(&self, key: &str, vars: &[(&str, &str)]) -> SharedString {
            match self.0.get(key) {
                Some(s) => crate::i18n::fill(s, vars).into(),
                None => key.to_string().into(),
            }
        }
        fn serve(&self, code: Option<&str>, params: Option<&Params>, fallback: &str) -> SharedString {
            let Some(key) = code.and_then(|c| crate::serve_text::SERVE_TEXT.iter().find(|(k, _)| *k == c)) else {
                return fallback.to_string().into();
            };
            let Some(template) = self.0.get(&format!("serve.{}", key.1)) else {
                return fallback.to_string().into();
            };
            let owned: Vec<(String, String)> = params
                .map(|p| p.iter().map(|(k, v)| (k.clone(), super::param_text(v))).collect())
                .unwrap_or_default();
            let vars: Vec<(&str, &str)> = owned.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
            crate::i18n::fill(template, &vars).into()
        }
        fn at_date(&self, ms: u64, _now: u64) -> SharedString {
            format!("at {}", crate::ui::format::hhmm(ms)).into()
        }
    }
}

#[cfg(test)]
pub(crate) mod fixtures {
    use crate::model::{Activity, Agent, Channel, Message, WorkspaceView};

    pub const MIN: u64 = 60_000;
    /// 2026-11-06T22:41:00Z.
    pub const NOW: u64 = 1_794_004_860_000;

    pub fn agent(key: &str, name: &str) -> Agent {
        Agent {
            key: key.into(),
            name: name.into(),
            kind: "teammate".into(),
            presence: "working".into(),
            state: "running".into(),
            last_action_ms: Some(NOW - MIN),
            ..Default::default()
        }
    }

    pub fn face(state: &str, detail: Option<&str>) -> Activity {
        Activity { state: state.into(), detail: detail.map(Into::into), since_ms: NOW, source_ms: NOW }
    }

    pub fn message(id: &str, agent: &str, ts_ms: u64) -> Message {
        Message {
            id: id.into(),
            agent: agent.into(),
            from: "agent".into(),
            author: agent.into(),
            ts_ms,
            text: format!("text {id}"),
            channels: vec!["orchard".into()],
            ..Default::default()
        }
    }

    pub fn channel(id: &str) -> Channel {
        Channel { id: id.into(), ..Default::default() }
    }

    pub fn view(agents: Vec<Agent>, messages: Vec<Message>) -> WorkspaceView {
        WorkspaceView { now_ms: NOW, agents, messages, ..Default::default() }
    }
}

#[cfg(test)]
mod tests {
    use super::fixtures::{MIN, NOW, agent, channel, face, message, view};
    use super::test_words::{en, es};
    use super::*;
    use crate::model::{Checkout, LobbyEntry, Shipment};

    fn keys(list: &[&Agent]) -> Vec<String> {
        list.iter().map(|a| a.key.clone()).collect()
    }

    // workspace.test.ts — selections

    #[test]
    fn a_channel_shows_top_level_messages_and_replies_open_in_their_thread_in_order() {
        let mut parent = message("p", "r", NOW - 30 * MIN);
        parent.label = Some("FINDING".into());
        parent.replies = vec!["r2".into(), "r1".into()];
        let mut r1 = message("r1", "r", NOW - 20 * MIN);
        r1.from = "lead".into();
        r1.reply_to = Some("p".into());
        let mut r2 = message("r2", "r", NOW - 10 * MIN);
        r2.from = "operator".into();
        r2.reply_to = Some("p".into());
        let v = view(vec![agent("r", "keychain-review-2")], vec![parent, r1, r2]);
        let ids = |l: Vec<&Message>| l.iter().map(|m| m.id.clone()).collect::<Vec<_>>();
        assert_eq!(ids(channel_stream(&v, "orchard")), ["p"]);
        assert_eq!(ids(thread(&v, "p")), ["r1", "r2"]);
        assert!(thread(&v, "nope").is_empty());
    }

    #[test]
    fn peer_chatter_and_protocol_records_stay_out_of_a_channel() {
        let mut peer = message("peer", "a", NOW);
        peer.from = "peer".into();
        let mut record = message("rec", "a", NOW);
        record.structured = Some("{}".into());
        let v = view(vec![], vec![peer, record, message("m", "a", NOW)]);
        assert_eq!(channel_stream(&v, "orchard").len(), 1);
    }

    #[test]
    fn threads_put_the_one_answered_last_first() {
        let mut a = message("a", "x", NOW - 50 * MIN);
        a.replies = vec!["a1".into()];
        let mut b = message("b", "x", NOW - 40 * MIN);
        b.replies = vec!["b1".into()];
        let a1 = message("a1", "x", NOW - MIN);
        let b1 = message("b1", "x", NOW - 30 * MIN);
        let v = view(vec![], vec![a, b, a1, b1]);
        let ids: Vec<&str> = threads(&v).iter().map(|m| m.id.as_str()).collect();
        assert_eq!(ids, ["a", "b"]);
    }

    #[test]
    fn the_sidebar_lists_agents_active_in_24_h_newest_first_eight_before_see_all() {
        let mut agents: Vec<Agent> = (0..10)
            .map(|i| {
                let mut a = agent(&format!("a{i}"), &format!("agent-{i}"));
                a.last_action_ms = Some(NOW - (i + 1) * 60 * MIN);
                a.activity = face("running", None);
                a
            })
            .collect();
        let mut old = agent("old", "old");
        old.presence = "finished".into();
        old.last_action_ms = Some(NOW - 25 * 60 * MIN);
        agents.push(old);
        let v = view(agents, vec![]);
        let s = sidebar_agents(&v, NOW);
        assert_eq!(keys(&s.shown), ["a0", "a1", "a2", "a3", "a4", "a5", "a6", "a7"]);
        assert_eq!(keys(&s.rest), ["a8", "a9"]);
    }

    #[test]
    fn open_sessions_go_on_their_own() {
        let mut s = agent("session:1", "Walter");
        s.kind = "session".into();
        let mut done = agent("session:2", "Old");
        done.kind = "session".into();
        done.state = "done".into();
        done.activity = face("running", None);
        let v = view(vec![s, done], vec![]);
        let side = sidebar_agents(&v, NOW);
        assert_eq!(keys(&side.sessions), ["session:1"]);
        assert_eq!(keys(&side.shown), ["session:2"]);
    }

    // inactive.test.ts

    fn quiet(key: &str, state: &str, quiet_min: u64) -> Agent {
        let mut a = agent(key, key);
        a.last_action_ms = Some(NOW - quiet_min * MIN);
        a.activity = face(state, None);
        a
    }

    #[test]
    fn a_closed_agent_moves_into_inactive_after_ten_minutes() {
        let closed = |m| {
            let mut a = quiet("c", "finished", m);
            a.closed = true;
            a.activity = face("finished", Some("closed"));
            a
        };
        assert!(!is_inactive(&closed(9), NOW));
        assert!(is_inactive(&closed(11), NOW));
    }

    #[test]
    fn idle_finished_or_lost_for_thirty_minutes_is_inactive_and_busy_never_is() {
        assert!(!is_inactive(&quiet("idle", "normal", 29), NOW));
        assert!(is_inactive(&quiet("idle", "normal", 31), NOW));
        assert!(is_inactive(&quiet("done", "finished", 31), NOW));
        let mut lost = quiet("lost", "normal", 31);
        lost.lost = true;
        assert!(is_inactive(&lost, NOW));
        for busy in ["thinking", "running", "waiting", "blocked"] {
            assert!(!is_inactive(&quiet(busy, busy, 120), NOW), "{busy}");
        }
    }

    #[test]
    fn the_sidebar_splits_them_working_agents_first() {
        let mut c = quiet("c", "finished", 15);
        c.closed = true;
        c.activity = face("finished", Some("closed"));
        let v = view(vec![quiet("w", "running", 1), quiet("b", "blocked", 90), quiet("i", "normal", 45), c], vec![]);
        let s = sidebar_agents(&v, NOW);
        assert_eq!(keys(&s.shown), ["w", "b"]);
        assert_eq!(keys(&s.inactive), ["c", "i"]);
    }

    // hidden.test.ts

    fn hidden_view() -> WorkspaceView {
        let mk = |key: &str, name: &str, hidden: bool, state: &str| {
            let mut a = agent(key, name);
            a.hidden = hidden;
            a.activity = face(state, None);
            a
        };
        view(
            vec![
                mk("rivet", "Owen", false, "editing"),
                mk("lima", "Laura", true, "blocked"),
                mk("nut", "Alice", true, "editing"),
                mk("coil", "Iris", false, "editing"),
            ],
            vec![],
        )
    }

    #[test]
    fn hidden_agents_leave_the_sidebar_and_list_by_name() {
        let v = hidden_view();
        let s = sidebar_agents(&v, NOW);
        let mut all = keys(&[s.shown, s.rest, s.inactive].concat());
        all.sort();
        assert_eq!(all, ["coil", "rivet"]);
        let names: Vec<&str> = hidden_agents(&v).iter().map(|a| a.name.as_str()).collect();
        assert_eq!(names, ["Alice", "Laura"]);
    }

    #[test]
    fn the_menu_item_says_hide_or_show_and_posts_the_matching_action() {
        let v = hidden_view();
        let w = en();
        assert_eq!(hide_action(&v.agents[0], &w), HideAction { label: "Hide".into(), action: "hide-agent" });
        assert_eq!(hide_action(&v.agents[1], &w), HideAction { label: "Show".into(), action: "show-agent" });
    }

    #[test]
    fn temporary_agents_are_their_own_group_and_never_in_the_sidebar() {
        let mut t = agent("t", "tmp");
        t.temporary = true;
        let mut th = agent("th", "tmp hidden");
        th.temporary = true;
        th.hidden = true;
        let v = view(vec![t, th, agent("a", "a")], vec![]);
        assert_eq!(keys(&temporary_agents(&v)), ["t"]);
        assert_eq!(sidebar_agents(&v, NOW).listed(), 1);
    }

    #[test]
    fn the_filter_matches_name_handle_or_team() {
        let mut a = agent("a", "Laura");
        a.team = Some("protocol".into());
        let mut b = agent("b", "Owen");
        b.handle = Some("rivet".into());
        let list = vec![&a, &b];
        assert_eq!(keys(&filter_agents(&list, "LAU")), ["a"]);
        assert_eq!(keys(&filter_agents(&list, "proto")), ["a"]);
        assert_eq!(keys(&filter_agents(&list, "riv")), ["b"]);
        assert_eq!(filter_agents(&list, "  ").len(), 2);
    }

    #[test]
    fn spanish_order_ignores_case_and_accents() {
        assert_eq!(collate("álvaro", "Bea"), Ordering::Less);
        assert_eq!(collate("nube", "ñu"), Ordering::Less);
        assert_eq!(collate("ñu", "oso"), Ordering::Less);
    }

    // rooms.test.ts — the sidebar's channels

    #[test]
    fn project_channels_list_no_rooms_and_keep_quiet_ones_under_more() {
        let mut a = channel("a");
        a.kind = Some("project".into());
        let mut b = channel("b");
        b.quiet = true;
        let mut c = channel("c");
        c.kind = Some("room".into());
        let mut old = channel("old");
        old.archived = true;
        let v = WorkspaceView { channels: vec![a, b, c, old], ..Default::default() };
        let s = sidebar_channels(&v);
        assert_eq!(s.shown.iter().map(|c| c.id.as_str()).collect::<Vec<_>>(), ["a"]);
        assert_eq!(s.more.iter().map(|c| c.id.as_str()).collect::<Vec<_>>(), ["b"]);
    }

    #[test]
    fn home_never_opens_on_a_room() {
        let mut first = channel("first");
        first.kind = Some("room".into());
        let v = WorkspaceView { channels: vec![first.clone(), channel("repo")], ..Default::default() };
        assert_eq!(default_channel_id(&v), Some("repo"));
        let only = WorkspaceView { channels: vec![first], ..Default::default() };
        assert_eq!(default_channel_id(&only), None);
    }

    #[test]
    fn a_built_in_room_can_be_opened_even_if_serve_did_not_list_it() {
        let v = WorkspaceView::default();
        assert!(find_channel(&v, "commons").is_some_and(|c| c.built_in && c.is_room()));
        assert!(find_channel(&v, "nope").is_none());
        assert_eq!(channel_route("lobby"), Route::Lobby);
    }

    // serve-text.test.ts — roleVars

    fn coded(name: &str, code: &str, params: &[(&str, &str)]) -> Agent {
        Agent {
            name: name.into(),
            name_code: Some(code.into()),
            name_params: Some(params.iter().map(|(k, v)| (k.to_string(), serde_json::Value::String(v.to_string()))).collect()),
            ..Default::default()
        }
    }

    #[test]
    fn a_known_role_is_worded_with_its_number() {
        let a = coded("keychain · reviewer 2", "name.team_role", &[("team", "keychain"), ("role", "reviewer"), ("number", "2")]);
        assert_eq!(agent_name_with(&a, &en()), "keychain · reviewer 2");
        assert_eq!(agent_name_with(&a, &es()), "keychain · revisor 2");
    }

    #[test]
    fn an_unnamed_session_and_a_chosen_name() {
        let a = coded("harbard · session 17:26", "name.session.project_time", &[("project", "harbard"), ("time", "17:26")]);
        assert_eq!(agent_name_with(&a, &en()), "harbard · session 17:26");
        assert_eq!(agent_name_with(&a, &es()), "harbard · sesión 17:26");
        let nora = Agent { name: "Nora".into(), ..Default::default() };
        assert_eq!(agent_name_with(&nora, &es()), "Nora");
        assert_eq!(shown_name_with("session ab12cd34", &es()), "sesión ab12cd34");
    }

    #[test]
    fn a_role_line_agrees_with_the_names_gender() {
        let mut p = Params::new();
        p.insert("role".into(), "reviewer".into());
        p.insert("gender".into(), "f".into());
        p.insert("team".into(), "protocol".into());
        let a = Agent {
            role_line: Some("reviewer · protocol".into()),
            role_line_code: Some("role_line".into()),
            role_line_params: Some(p),
            ..Default::default()
        };
        assert_eq!(agent_role_line(&a, &en()), "reviewer · protocol");
        assert_eq!(agent_role_line(&a, &es()), "revisora · protocol");
        let mut solo = Params::new();
        solo.insert("role".into(), "builder".into());
        solo.insert("gender".into(), "m".into());
        let b = Agent {
            role_line: Some("builder".into()),
            role_line_code: Some("role_line.solo".into()),
            role_line_params: Some(solo),
            ..Default::default()
        };
        assert_eq!(agent_role_line(&b, &es()), "constructor");
        let free = Agent { role_line: Some("the ledger · x".into()), ..Default::default() };
        assert_eq!(agent_role_line(&free, &es()), "the ledger · x");
    }

    #[test]
    fn a_local_role_keeps_its_number_and_leaves_unknown_ones() {
        assert_eq!(local_role("reviewer 2", &es()), "revisor 2");
        assert_eq!(local_role("reviewer", &es()), "revisor");
        assert_eq!(local_role("constructor", &es()), "constructor");
        assert_eq!(local_role("reviewer two", &es()), "reviewer two");
    }

    // legible-header.test.ts — headline and places

    /// Local midnight of NOW's day plus `h:m`, so "today" is stable in any zone.
    fn local_at(h: u64, m: u64) -> u64 {
        let (hh, mm) = crate::ui::format::local_hm(NOW);
        let midnight = NOW - (hh as u64 * 60 + mm as u64) * MIN - NOW % MIN;
        midnight + (h * 60 + m) * MIN
    }

    fn header_view() -> (WorkspaceView, Channel) {
        let now = local_at(22, 15);
        let mk = |key: String, last: u64, presence: &str| Agent {
            key: key.clone(),
            name: key,
            presence: presence.into(),
            last_action_ms: Some(now - last * MIN),
            ..Default::default()
        };
        let mut agents: Vec<Agent> = (0..36).map(|i| mk(format!("cap{i}"), 10 + i, "finished")).collect();
        agents.push(mk("cap86fix".into(), 1, "working"));
        agents.push(mk("old".into(), 23 * 60, "finished"));
        let repo = "/Users/p/Projects/acme";
        let acme = Channel {
            id: "acme".into(),
            repo: repo.into(),
            checkouts: vec![
                Checkout { path: repo.into(), branch: Some("main".into()) },
                Checkout { path: format!("{repo}/.worktrees/cli-feedback"), branch: Some("feat/cli-feedback".into()) },
                Checkout { path: format!("{repo}/.worktrees/cli-flow-progress"), branch: Some("feat/cli-flow-progress".into()) },
            ],
            members: agents.iter().map(|a| a.key.clone()).collect(),
            recent: agents.iter().map(|a| a.key.clone()).collect(),
            active_ms: Some(now - MIN),
            ..Default::default()
        };
        let v = WorkspaceView { now_ms: now, agents, channels: vec![acme.clone()], ..Default::default() };
        (v, acme)
    }

    #[test]
    fn sessions_today_count_from_local_midnight_and_who_works_now() {
        let (v, acme) = header_view();
        let head = channel_headline(&v, &acme, &en());
        assert_eq!(head.sessions, "37 sessions today");
        assert_eq!(head.now.as_deref(), Some("1 working now"));
        assert_eq!(head.working, 1);
        assert!(!head.agents.iter().any(|a| a.key == "old"));
    }

    #[test]
    fn nobody_working_says_so_and_nobody_today_falls_back_to_24_h() {
        let (mut v, acme) = header_view();
        let late = Channel { recent: vec!["old".into()], ..acme.clone() };
        assert_eq!(channel_headline(&v, &late, &en()).sessions, "1 session in the last 24 h");
        for a in &mut v.agents {
            a.presence = "finished".into();
        }
        assert_eq!(channel_headline(&v, &acme, &en()).now.as_deref(), Some("nobody working now"));
        let one = WorkspaceView { agents: vec![Agent { key: "a".into(), last_action_ms: Some(v.now_ms - MIN), ..Default::default() }], ..v.clone() };
        let ch = Channel { recent: vec!["a".into()], ..acme };
        assert_eq!(channel_headline(&one, &ch, &en()).sessions, "1 session today");
    }

    #[test]
    fn with_nobody_recent_the_line_says_when_it_last_had_activity_or_who_read_it() {
        let (v, acme) = header_view();
        let idle = Channel { recent: vec![], ..acme.clone() };
        let head = channel_headline(&v, &idle, &en());
        assert!(head.sessions.starts_with("Nobody's working here now · last activity"));
        assert_eq!(head.now.as_deref(), Some("1 working now"));
        let read = Channel { recent: vec![], active_ms: None, members: vec!["cap0".into()], ..acme };
        assert_eq!(channel_why(&v, &read, &en()).line, "Nobody has written here: one agent read it with git");
    }

    #[test]
    fn paths_and_branches_go_behind_the_info_button() {
        let (_, acme) = header_view();
        let places = channel_places(&acme, &en());
        assert_eq!(places.title, "3 copies of ~/Projects/acme");
        assert_eq!(places.copies[0], Place { path: "main copy".into(), full: acme.repo.clone(), branch: Some("main".into()) });
        assert_eq!(places.copies[1].path, ".worktrees/cli-feedback");
        let elsewhere = Channel {
            checkouts: vec![Checkout { path: "/Users/p/elsewhere/acme-copy".into(), branch: None }],
            ..acme
        };
        let one = channel_places(&elsewhere, &en());
        assert_eq!(one.copies[0].path, "~/elsewhere/acme-copy");
        assert_eq!(one.title, "~/Projects/acme");
    }

    // The two-writers strip, workspaces, deliveries, lobby

    #[test]
    fn the_two_writers_strip_names_the_agents_and_the_checkout() {
        let v = view(vec![agent("k", "keychain"), agent("r", "keychain-review-2")], vec![]);
        let warning = WriterWarning { checkout: "/work/orchard-wt-identity/".into(), agents: vec!["k".into(), "r".into()], since_ms: NOW };
        assert_eq!(warning_lead(&warning, &en()), "Two agents in the same checkout:");
        let text = warning_text(&v, &warning, &en());
        assert!(text.starts_with("keychain, keychain-review-2 writing in orchard-wt-identity since "), "{text}");
        let three = WriterWarning { agents: vec!["a".into(), "b".into(), "c".into()], ..warning };
        assert_eq!(warning_lead(&three, &en()), "3 agents in the same checkout:");
    }

    #[test]
    fn the_head_names_the_current_workspace_and_the_busiest_other_one() {
        let tile = |id: &str, current: bool, working: u64| WorkspaceTile { id: id.into(), name: id.to_uppercase(), current, working, ..Default::default() };
        assert_eq!(current_name(&[]), "harbard");
        let tiles = vec![tile("a", true, 9), tile("b", false, 2), tile("c", false, 5), tile("d", false, 5)];
        assert_eq!(current_name(&tiles), "A");
        assert_eq!(working_elsewhere(&tiles).map(|t| t.id.as_str()), Some("c"));
        assert_eq!(working_in_text(&tiles[2], &en()), "5 working in C");
        assert!(working_elsewhere(&[tile("a", true, 3), tile("b", false, 0)]).is_none());
    }

    #[test]
    fn deliveries_and_the_lobby_count_what_the_sidebar_shows() {
        let ship = |pending| Shipment { pending, ..Default::default() };
        let entry = |p: &str| LobbyEntry { presence: p.into(), ..Default::default() };
        let v = WorkspaceView {
            shipments: vec![ship(true), ship(false), ship(true)],
            lobby: vec![entry("live"), entry("idle"), entry("stale"), entry("ended"), entry("live")],
            ..Default::default()
        };
        assert_eq!(pending_count(&v), 2);
        assert_eq!(lobby_counts(&v), LobbyCounts { here: 2, around: 2, gone: 1 });
        assert_eq!(lobby_here_count(&v), 2);
    }

    #[test]
    fn short_paths_and_file_names_follow_the_console() {
        assert_eq!(short_path("/Users/x/Projects/a"), "~/Projects/a");
        assert_eq!(short_path("/work/a"), "/work/a");
        assert_eq!(file_name("/work/orchard-wt/"), "orchard-wt");
        assert_eq!(file_name("x"), "x");
    }
}
