//! Command-line surface. Shared context/output options are global so they may
//! appear before or after the verb; each verb adds only its own flags.

use clap::{Args, Parser, Subcommand};

#[derive(Args, Debug, Clone)]
#[group(required = true, multiple = false)]
pub struct LeaseTarget {
    /// Lease the current Git worktree for a single writer.
    #[arg(long)]
    pub writer: bool,

    /// Lease one host-global service name.
    #[arg(long, value_name = "NAME")]
    pub service: Option<String>,
}

#[derive(Parser, Debug)]
#[command(
    name = "harbard",
    version,
    about = "cross-harness session bus — presence, intent, and notes between agent sessions",
    disable_help_subcommand = true
)]
pub struct Cli {
    #[command(flatten)]
    pub common: Common,
    #[command(subcommand)]
    pub cmd: Cmd,
}

/// Options shared by every verb (global: valid anywhere on the line).
#[derive(Args, Debug, Clone)]
pub struct Common {
    /// Session id (else $HARBARD_SESSION, hook stdin, $CLAUDE_CODE_SESSION_ID, fallback).
    #[arg(long, global = true, value_name = "ID")]
    pub session: Option<String>,

    /// Harness: claude-code | codex | other.
    #[arg(long, global = true, value_name = "HARNESS")]
    pub harness: Option<String>,

    /// Read hook context (session_id, cwd) from stdin JSON for this harness.
    #[arg(long, global = true, value_name = "HARNESS")]
    pub hook: Option<String>,

    /// Working directory (else hook stdin, else the process cwd).
    #[arg(long, global = true, value_name = "PATH")]
    pub cwd: Option<String>,

    /// Repository path; filters `ls` and selects the checkout inspected by `audit`.
    #[arg(long, global = true, value_name = "PATH")]
    pub repo: Option<String>,

    /// Branch name (else derived from the repo's HEAD).
    #[arg(long, global = true, value_name = "NAME")]
    pub branch: Option<String>,

    /// Role: main | herd | headless.
    #[arg(long, global = true, value_name = "ROLE")]
    pub role: Option<String>,

    /// Store home (else $HARBARD_HOME, else ~/.harbard).
    #[arg(long, global = true, value_name = "PATH")]
    pub home: Option<String>,

    /// Emit machine-readable JSON on stdout.
    #[arg(long, global = true)]
    pub json: bool,

    /// Liveness threshold in seconds (default 900).
    #[arg(long, global = true, value_name = "SECS")]
    pub stale_secs: Option<u64>,

    /// Days of history serve keeps in memory for live projections (default 14, minimum 7). The event log on disk is never deleted.
    #[arg(long, global = true, value_name = "DAYS")]
    pub retention_days: Option<u64>,
}

#[derive(Subcommand, Debug)]
pub enum Cmd {
    /// Announce session start; runs lazy GC. Never refuses.
    Hello {
        /// Declared work for this session.
        #[arg(long, value_name = "TEXT")]
        intent: Option<String>,
    },
    /// Introduce this session with a proper name that has character. May be done once.
    ///
    /// Pick the kind of name a person or a character could carry, inspired by
    /// your mission; not a common noun or a tool or role word. One
    /// English-readable word, 3-12 letters, easy to say aloud. Good: Ottoline,
    /// Bramwell, Vesper. Bad: Wrench, Helper, Conduit, Agent.
    ///
    /// harbard refuses generic words, names held by another session and names
    /// any session carried in the last 90 days, and says why, so choose again.
    /// The register is one shared namespace across every harness and project on
    /// this machine, read by people scanning a board: a callsign is an address,
    /// not prose, so it stays English-readable whatever language the work uses.
    Name {
        #[arg(value_name = "NAME")]
        name: String,
        /// With a name from the given-name list (Nora, Owen, Iris…):
        /// one line saying why, shown in the console's Profile.
        #[arg(long, value_name = "TEXT")]
        reason: Option<String>,
        /// With a name from the given-name list: your team, said outright
        /// (`cli`, `robots`). It wins over what the console would derive.
        #[arg(long, value_name = "TEAM")]
        team: Option<String>,
    },
    /// Update declared work. Latest per session wins.
    Intent {
        #[arg(value_name = "TEXT")]
        text: String,
    },
    /// Leave a note; broadcast by default.
    Note {
        /// What the note is for. `warning`, `ask` and `handoff` wake an idle
        /// reader (through `harbard watch`) or land at its next prompt; `fyi`
        /// waits for their next turn and never wakes anyone. A `handoff` goes
        /// to one live session, or to the operator (`--to harness:human`).
        #[arg(long, value_parser = ["fyi", "warning", "handoff", "ask"], default_value = "fyi")]
        kind: String,
        #[arg(value_name = "TEXT")]
        text: String,
        /// Recipient: `harness:<h>` | `session:<id>` | `repo:<path>`.
        #[arg(long = "to", value_name = "TARGET")]
        to: Option<String>,
        /// Reply to a prior note (its event id). Nested one level in the console.
        #[arg(long = "ref", alias = "reply", value_name = "NOTE_ID")]
        note_ref: Option<String>,
        /// One fact, as `key=value`. Repeatable; order is kept.
        ///
        /// Composes the house report shape: TEXT becomes the summary line and
        /// each field becomes its own `key: value` line under it. Asked for
        /// progress, ETA, blockers and proposals, four agents once answered in
        /// four different layouts and none could be compared — the shape exists
        /// so a reader can read across answers instead of into each one.
        ///
        /// It also decides your own preview: a note with a short first line is
        /// quoted by that line in every brief, instead of being clipped at 140
        /// bytes mid-word like nine notes in ten.
        #[arg(long = "field", value_name = "KEY=VALUE")]
        fields: Vec<String>,
        /// The technical detail, from a file: TEXT stays the first line the
        /// operator reads, and the file follows after a blank line.
        #[arg(long, value_name = "PATH")]
        body_file: Option<std::path::PathBuf>,
        /// Copy a file for the operator (`harbard attach`) and cite it in
        /// the note. Repeatable: `--attach a.png --attach b.md`.
        #[arg(long = "attach", value_name = "PATH")]
        attach: Vec<std::path::PathBuf>,
    },
    /// Answer a note: `harbard reply <note-id> "…"`, or `harbard reply last "…"`.
    ///
    /// Threads the answer under the note (`--ref`) and addresses its author:
    /// The operator (`harness:human`) or the session that wrote it. Works out
    /// who is answering from the environment, else from the session the note
    /// was addressed to. `last` takes the first answer `harbard owed` lists
    /// (the operator's first, then the oldest), else the newest note another
    /// session addressed to you; never a note relayed to an agent.
    Reply {
        /// Note id (26 chars, or an unambiguous prefix), or `last`.
        target: String,
        #[arg(value_name = "TEXT")]
        text: String,
        /// `fyi` by default; `ask` when your answer asks something back.
        #[arg(long, value_parser = ["fyi", "warning", "ask"], default_value = "fyi")]
        kind: String,
        /// One fact, as `key=value`, as in `note --field`. Repeatable.
        #[arg(long = "field", value_name = "KEY=VALUE")]
        fields: Vec<String>,
        /// On a note relayed to your agents (`to_agent:` lines): whose answer
        /// this is. Its name prefixes the reply; without it the answer is
        /// yours, unsigned.
        #[arg(long, value_name = "NAME")]
        agent: Option<String>,
    },
    /// Notes for your in-process agents (`to_agent:` lines) that you have not
    /// relayed yet, each ready to paste into SendMessage. After sending one,
    /// `harbard relay --sent <note-id>` marks it relayed; `harbard reply
    /// <note-id> --agent <name> "…"` later carries the agent's answer back
    /// under its name.
    Relay {
        /// Mark this note relayed (an ack, so it is no longer owed).
        #[arg(long, value_name = "NOTE_ID")]
        sent: Option<String>,
        /// At most this many notes.
        #[arg(long, value_name = "N", default_value_t = 20)]
        limit: usize,
    },
    /// Hint of a clean end. Idempotent. Also releases this session's writer lease.
    Bye,
    /// PreToolUse writer-lease gate: classify the pending tool call, acquire or
    /// validate the checkout lease before any mutating operation runs.
    Gate {
        /// Override harness supervisor pid (tests / recovery). Omit to auto-bind
        /// Claude Code / Codex / Cursor via ancestry — never pass `$$` from a tool shell.
        #[arg(long, value_name = "PID")]
        pid: Option<u32>,
    },
    /// Classify a shell command against the checkout lifecycle: does it move
    /// HEAD, mutate a worktree, hide behind a Git alias, or reach into another
    /// checkout? Read-only — takes no lease and records nothing.
    CheckoutPolicy {
        /// The command to classify, exactly as the harness would run it.
        #[arg(long, value_name = "COMMAND")]
        command: String,
    },
    /// List sessions as live, idle (quiet running process), stale, or ended.
    Ls {
        /// Scan every segment, not just today+yesterday.
        #[arg(long)]
        all: bool,
    },
    /// Your own team: the teammates and subagents of this lead session, with
    /// their workshop name, role and team, state and since when, their last
    /// message line, and whether a DONE is waiting for you.
    ///
    /// `harbard ls` lists sessions; an in-process teammate is not one, so a
    /// lead running fifteen agents could see none of them from the terminal.
    Agents {
        /// Every lead's agents, not only this session's.
        #[arg(long)]
        all: bool,
        /// Redraw until interrupted (one JSON line per frame with `--json`).
        #[arg(long)]
        watch: bool,
        /// Seconds between frames with `--watch`.
        #[arg(long, value_name = "SECS", default_value_t = 5)]
        interval_secs: u64,
        /// Stop after this many frames (tests).
        #[arg(long, hide = true, value_name = "N")]
        frames: Option<u64>,
    },
    /// What you owe, in one bounded list ordered by urgency, each item with
    /// the exact command that settles it: answers to the operator, notes to
    /// relay to an agent, BLOCKED agents, answers to peers, DONEs waiting
    /// for you and agents that went quiet.
    Owed {
        /// At most this many items (the total is always reported).
        #[arg(long, value_name = "N", default_value_t = 20)]
        limit: usize,
    },
    /// The whole board in one object: sessions, checkouts, rooms, obligations.
    ///
    /// The same projection `serve` hands the console (SPEC-FLEET-VIEW §1), so
    /// what an agent reads on the terminal and what the operator sees on the
    /// screen are the same numbers rather than two derivations of them.
    Fleet,
    /// Acknowledge a note you received: tells its author you have it.
    ///
    /// Distinct from the delivery receipt the store infers. "It reached you"
    /// is a fact about a cursor; "I have it" is a claim only you can make.
    Ack {
        /// Event id to acknowledge, or `last` for the oldest handoff or ask
        /// you still owe an answer (addressed to your session first). `last`
        /// refuses when nothing is owed.
        target: String,
        /// Your answer. Also posted as a `--ref` reply to the acked note, so
        /// it reads as a message rather than a receipt.
        #[arg(long)]
        note: Option<String>,
    },
    /// Post-join unread notes; requires stable session identity, advances shared delivery position.
    Inbox,
    /// Explicit cross-session dependencies and durable continuation checkpoints.
    Dependency {
        #[command(subcommand)]
        action: DependencyCmd,
    },
    /// Wait without polling anyone yourself: `await <dependency-id>`,
    /// `await lane <lane>` (until you own it), or `await pr <n> merged|closed`.
    Await {
        /// A dependency id, or `lane <lane>`, or `pr <number> merged|closed`
        /// (`closed` = no longer open, merged included; `merged` fails with
        /// exit 4 if the PR is closed unmerged).
        #[arg(num_args = 1..=3, required = true, value_name = "TARGET")]
        target: Vec<String>,
        /// With `await lane`: what you need the lane for, if not queued yet.
        #[arg(long = "for", value_name = "TEXT")]
        purpose: Option<String>,
        /// With `await lane`: the ttl your turn starts with (`90s`, `60m`, `2h`).
        #[arg(long, value_name = "DURATION", default_value = "60m")]
        ttl: String,
        /// Maximum time to keep this invocation open; timeout preserves the
        /// dependency, the queue position, or the PR interest.
        #[arg(long, default_value_t = 300, value_parser = clap::value_parser!(u64).range(0..=86_400))]
        timeout_secs: u64,
        /// Local log polling interval, independent of model turns.
        #[arg(long, default_value_t = 250, value_parser = clap::value_parser!(u64).range(10..=60_000))]
        poll_ms: u64,
    },
    /// Turns on shared resources, one owner at a time (migrations, dev-stack,
    /// delivery-dev|pre|prod, provider-config). Scoped to the repository, so
    /// every worktree of it sees the same lanes.
    Lane {
        #[command(subcommand)]
        action: LaneCmd,
    },
    /// Keep your lane(s): restart the ttl of one lane, or of every lane you
    /// own in this repository.
    Heartbeat {
        #[arg(value_name = "LANE")]
        lane: Option<String>,
    },
    /// Copy files for the operator to see into `{home}/attachments/<ulid>/`
    /// and print the paths to cite in a message. png, jpg, webp, gif, md,
    /// html, txt, json, diff, patch, log or huginn; 25 MB each; no symlinks.
    Attach {
        #[arg(value_name = "PATH", required = true)]
        paths: Vec<std::path::PathBuf>,
        /// before | after | report | code. A before and an after image
        /// in one message show side by side.
        #[arg(long, value_name = "LABEL")]
        label: Option<String>,
    },
    /// Views: components an agent shows the operator instead of a sentence.
    Huginn {
        #[command(subcommand)]
        action: HuginnAction,
    },
    /// Canonical agent protocol card. With --hook (SessionStart) it prints a
    /// one-line pointer to the card instead.
    Protocol {
        /// Print the full card instead of the compact one (or the hook pointer).
        #[arg(long)]
        full: bool,
        /// Print what a starting subagent is told about its reports.
        #[arg(long, conflicts_with = "full")]
        subagent: bool,
    },
    /// MCP stdio facade over protocol v1 verbs (tools + protocol resource).
    ///
    /// Hosts launch `harbard mcp` as a subprocess. Newline-delimited JSON-RPC;
    /// tools shell out to this same binary. Not a second bus — a host adapter.
    Mcp,
    /// Stable finite machine call. Always emits one harbard/cli/v1 JSON envelope.
    Call {
        /// Operation id from `harbard protocol --json` (for example `note`).
        operation: String,
        /// Operation arguments as one JSON object.
        #[arg(long, value_name = "JSON", default_value = "{}")]
        input: String,
    },
    /// Subscribe to append-only bus events as harbard/stream/v1 JSON Lines.
    Subscribe {
        /// Resume strictly after this event id. Unknown ids fail before follow.
        #[arg(long, value_name = "EVENT_ID")]
        after: Option<String>,
        /// Drain the current snapshot and exit after an `end` frame.
        #[arg(long)]
        once: bool,
        /// Poll interval while following.
        #[arg(long, value_name = "MILLISECONDS", default_value_t = 250)]
        poll_ms: u64,
        /// Emit a heartbeat after this idle interval; 0 disables heartbeats.
        #[arg(long, value_name = "MILLISECONDS", default_value_t = 15_000)]
        heartbeat_ms: u64,
        /// Restrict to event kinds (repeatable: note, ack, intent, ...).
        #[arg(long = "event", value_name = "KIND")]
        events: Vec<String>,
        /// Only events addressed to this session/harness/repository.
        #[arg(long)]
        addressed: bool,
    },
    /// THE hook verb: a bounded digest of live sessions and unread notes.
    Brief {
        /// Also record a liveness heartbeat and advance the brief high-water mark.
        #[arg(long)]
        touch: bool,
        /// Inline channel (F2): deliver ONLY notes addressed to me, for the hot
        /// PreToolUse path. Keeps its own mark, so the turn-boundary brief is
        /// unaffected. Implies a mark advance — the message is delivered once.
        #[arg(long)]
        inline: bool,
        /// With `--inline`, skip the store read when the last check is newer
        /// than this many seconds. `0` disables throttling.
        #[arg(long, value_name = "SECS", default_value_t = 0)]
        throttle: u64,
    },
    /// Stop hook (background): wait, without a turn, until something this
    /// session must hear lands, then wake it with the whole message.
    ///
    /// Wakes for warnings, asks and handoffs addressed to this session (and
    /// warnings or asks to its harness or repository), replies to its own
    /// notes, and results on dependencies it opened. Exit 2 with the message on
    /// stderr; exit 0 and silence on timeout, session end, or nothing to say.
    Watch {
        /// Longest the watcher stays armed.
        #[arg(long, value_name = "SECS", default_value_t = crate::watch::LIFETIME_SECS)]
        lifetime_secs: u64,
        /// Tail poll interval.
        #[arg(long, hide = true, default_value_t = crate::watch::POLL_MS, value_parser = clap::value_parser!(u64).range(10..=60_000))]
        poll_ms: u64,
        /// How long a burst is collected into one wake.
        #[arg(long, hide = true, default_value_t = crate::watch::COALESCE_MS)]
        coalesce_ms: u64,
        /// Codex: arm a detached watcher that delivers through `codex queue`,
        /// and return at once.
        #[arg(long)]
        detach: bool,
        /// Deliver through `codex queue --thread <session>` instead of exit 2.
        #[arg(long, hide = true, value_parser = ["codex"])]
        deliver: Option<String>,
        /// Harness process whose exit ends the watcher.
        #[arg(long, hide = true, value_name = "PID")]
        supervisor_pid: Option<u32>,
    },
    /// Stop hook: hold for current asks/handoffs created after this session joined.
    ///
    /// Records the turn end for watch. Reading via inbox is not an answer;
    /// prints block JSON until reply/ack (at most twice), otherwise silence.
    StopCheck,
    /// Offer a commit for integration: the queue for the right to move a branch.
    ///
    /// Nobody waits for a checkout. An agent works wherever it likes, commits,
    /// and hands the SHA over; `integrate` decides what actually lands.
    Deliver {
        /// The exact commit being offered. Resolved immediately, because a
        /// name can move between the offer and the integration.
        #[arg(long, value_name = "SHA")]
        sha: String,
        /// Branch this wants to move.
        #[arg(long, value_name = "BRANCH", default_value = "main")]
        branch: String,
        /// Shell command that decides whether it may land. Runs in the
        /// candidate worktree, never in yours.
        #[arg(long, value_name = "COMMAND", default_value = "")]
        checks: String,
        /// Command that makes the candidate worktree runnable before the checks
        /// touch it. A fresh worktree has tracked files and nothing else — no
        /// node_modules, no target, no .venv — so anything that imports a
        /// workspace package needs this. Its failure is reported as `failed`,
        /// never as a rejection of your commit.
        /// Example: --prepare 'ln -s ../../node_modules node_modules'
        #[arg(long, value_name = "COMMAND", default_value = "")]
        prepare: String,
        /// Repository. Defaults to the current directory's Git root.
        #[arg(long, value_name = "PATH")]
        repo: Option<String>,
    },
    /// Drain the delivery queue: build each candidate on the CURRENT tip, run
    /// its checks, and land only the commit that passed them.
    Integrate {
        /// Repository. Defaults to the current directory's Git root.
        #[arg(long, value_name = "PATH")]
        repo: Option<String>,
    },
    /// What the gate is costing this fleet, read from the admission log.
    ///
    /// Answers without asking anyone: who is blocked and by whom, how long
    /// waits actually last, and which calls the classifier cannot read — that
    /// last list is what to teach it next.
    Friction {
        /// How many rows per ranking.
        #[arg(long, value_name = "N", default_value_t = 10)]
        limit: usize,
    },
    /// What is queued, and what every past integration decided.
    Deliveries {
        /// Repository. Defaults to the current directory's Git root.
        #[arg(long, value_name = "PATH")]
        repo: Option<String>,
    },
    /// Subscribe to a named channel (F3). Anchored to this checkout by default,
    /// so every session that ever works here receives it.
    Join {
        /// Channel name; a leading `#` is accepted and ignored.
        channel: String,
        /// What the subscription follows.
        #[arg(long = "as", value_parser = ["repo", "session", "harness"], default_value = "repo")]
        anchor: String,
    },
    /// Unsubscribe from a named channel. The channel itself survives.
    Leave {
        channel: String,
        #[arg(long = "as", value_parser = ["repo", "session", "harness"], default_value = "repo")]
        anchor: String,
    },
    /// List Rooms, their members, and which ones reach me.
    #[command(visible_alias = "rooms")]
    Channels {
        /// Retired: orphan create is gone. Prefer `harbard join <name>`
        /// (creates + subscribes). Kept so old scripts get a clear redirect.
        #[arg(long)]
        create: Option<String>,
        /// Archive an empty channel (hide from the default rail; events stay).
        #[arg(long)]
        archive: Option<String>,
        /// Restore an archived channel to the rail.
        #[arg(long)]
        unarchive: Option<String>,
        /// Permanently remove an empty archived channel name. Events stay.
        #[arg(long)]
        delete: Option<String>,
        /// Include archived channels in the listing.
        #[arg(long)]
        all: bool,
    },
    /// Detect present harnesses and wire them in one idempotent pass.
    ///
    /// `--harness claude-code|codex|cursor` wires that one alone. Prefer this
    /// over triple `install`. Low-level escape hatch remains
    /// `install <target> --config PATH`.
    Setup {
        /// Plan only — detect, show the action per harness and every file the
        /// run would write or back up; write nothing.
        #[arg(long)]
        dry_run: bool,
    },
    /// Wire a single harness's hooks to harbard verbs (escape hatch).
    Install {
        /// Target harness.
        #[arg(value_parser = ["claude", "codex", "cursor"])]
        target: String,
        /// Print the exact hook JSON without writing anything.
        #[arg(long)]
        dry_run: bool,
        /// Config file to write (else the harness default path).
        #[arg(long, value_name = "PATH")]
        config: Option<String>,
        /// Remove every harbard-managed hook instead; foreign hooks stay.
        #[arg(long, conflicts_with = "dry_run")]
        uninstall: bool,
    },
    /// Full-text search over note bodies (local FTS5; same log, better index).
    Search {
        /// Free-text query (tokens AND'd; prefix match).
        query: String,
        /// Max hits (default 16, hard cap 64).
        #[arg(long, default_value_t = 16)]
        limit: usize,
        /// Rebuild the disposable index from JSONL before searching.
        #[arg(long)]
        rebuild: bool,
    },
    /// Read a whole conversation: a note and every reply under it.
    ///
    /// `--ref` could always CREATE a thread and the console renders one, but
    /// nothing could read one back — `inbox` is a one-shot cursor read, `search`
    /// needs a phrase you never saw, and `trace` answers about one note's
    /// delivery. So a conversation could not be copied out, and an agent
    /// replying into one could not see what its peers had already answered.
    ///
    /// Takes any id in the thread, not just the first: the message in front of
    /// you is as likely to be a reply as the question.
    Thread {
        /// Any note id in the conversation (26 chars, or an unambiguous prefix).
        id: String,
    },
    /// Store health: segments, corrupt-line counters, cursors, gc status.
    Doctor,
    /// Free the disk finished agents leave behind: their build dirs under
    /// `~/.cache/harbard*` and their `harbard-wt-*` worktrees.
    ///
    /// Lists each with its size; removes them only with `--yes`. Never an
    /// active agent's, never a worktree with uncommitted work, never the
    /// source checkout, `~/.harbard` or anything outside those two places.
    Clean {
        /// Agents the console shows as Closed or finished (the only mode).
        #[arg(long)]
        finished: bool,
        /// Remove what is listed. Without it, nothing is touched.
        #[arg(long)]
        yes: bool,
    },
    /// Audit Git checkout and worktree hygiene without changing the repository.
    /// Defaults to the cwd's git root when `--repo` is omitted.
    Audit {
        /// Compare each worktree with this ref (default: its upstream, then origin/dev).
        #[arg(long, value_name = "REF")]
        target: Option<String>,
        /// Inspect only this worktree. Linked lanes are included by default:
        /// hiding them behind a flag is how three abandoned ones went unnoticed
        /// for a day — the thing that accumulates must be the thing you see.
        #[arg(long)]
        primary_only: bool,
        /// Accepted and ignored: linked worktrees are now always included.
        /// Kept because external worktree lifecycle scripts pass it, and a flag
        /// that becomes the default should not break its callers.
        #[arg(long, hide = true)]
        include_worktrees: bool,
    },
    /// Acquire an authoritative writer or service lease.
    Claim {
        #[command(flatten)]
        target: LeaseTarget,
        /// Lease duration before a stale claimant may replace it (default 900).
        #[arg(long, value_name = "SECS", default_value_t = 900)]
        ttl_secs: u64,
        /// Supervised process id whose death makes the lease stale.
        /// Writer claims omit this to auto-bind Claude Code / Codex / Cursor via
        /// ancestry (never `$$` from a tool shell). Service claims never infer one.
        #[arg(long, value_name = "PID")]
        pid: Option<u32>,
    },
    /// Inspect a lease and optionally renew a matching active claim.
    Check {
        #[command(flatten)]
        target: LeaseTarget,
        /// Fencing token to compare with the current holder.
        #[arg(long, value_name = "ULID")]
        claim_id: Option<String>,
        /// Extend a matching active claim by its original TTL.
        #[arg(long, requires = "claim_id")]
        renew: bool,
        /// Explicit live supervisor PID. Writer renewal normally infers it
        /// from Claude Code, Codex, or Cursor ancestry.
        #[arg(long, value_name = "PID", requires = "renew")]
        pid: Option<u32>,
    },
    /// Release a matching lease; a different claim id is always fenced out.
    Release {
        #[command(flatten)]
        target: LeaseTarget,
        /// Fencing token returned by `claim`. Optional when the caller is the
        /// holder: an owner having to dig its own id out of the store to let go
        /// is how leases outlive the sessions that took them.
        #[arg(long, value_name = "ULID")]
        claim_id: Option<String>,
        /// Explicit live supervisor PID. Writer release normally infers it
        /// from Claude Code, Codex, or Cursor ancestry.
        #[arg(long, value_name = "PID")]
        pid: Option<u32>,
    },
    /// Serve the HTTP API (see docs/SPEC-API.md). Default bind 127.0.0.1.
    Serve {
        /// TCP port (default 7419; 0 picks a free port).
        #[arg(long, value_name = "PORT")]
        port: Option<u16>,
        /// Bind: `loopback` (default) or `tailnet` (loopback plus the address
        /// in `{home}/tailnet.json`, when set). Never a wildcard; `lan` is
        /// the retired name of `tailnet`.
        #[arg(long, value_name = "MODE", default_value = "loopback")]
        bind: String,
        /// Hub shared secret. Required for `--bind tailnet`. Also read from
        /// `HARBARD_HUB_TOKEN` or `~/.harbard/hub.token` when omitted.
        #[arg(long, value_name = "TOKEN")]
        token: Option<String>,
    },
    /// Turn this machine into a hub (token + launchd).
    Hub {
        #[command(subcommand)]
        action: HubCmd,
    },
    /// Pair this machine to a hub with one invite line from `harbard hub invite`.
    Pair {
        /// Invite: `http://<hub-ip>:7419#<token>` (omit to show status).
        invite: Option<String>,
        /// Friendly name on the fleet card (default: hostname short).
        #[arg(long)]
        name: Option<String>,
        /// Skip wiring claude / codex / cursor hooks.
        #[arg(long)]
        no_install: bool,
        /// Skip writing/starting the `com.harbard.satellite` LaunchAgent.
        #[arg(long)]
        no_daemon: bool,
        /// Forget pairing: remove satellite.json and stop the satellite agent.
        #[arg(long)]
        forget: bool,
    },
    /// Device identity and hub pairing (M4a).
    Device {
        #[command(subcommand)]
        action: DeviceCmd,
    },
    /// Run as a satellite and publish its heartbeat.
    Satellite {
        /// Heartbeat interval in seconds (default 15).
        #[arg(long, default_value_t = 15)]
        interval: u64,
    },
    /// Reconstruct the delivery pipeline from the ops log: where a note went,
    /// and which stage said no when it went nowhere.
    Trace {
        /// Trace one note id (accepts a prefix).
        #[arg(long, value_name = "ID")]
        note: Option<String>,
        /// Trace one session's recent hook/delivery activity (id or prefix).
        #[arg(long, value_name = "ID")]
        session: Option<String>,
    },
    /// Export durable learning facts (note cycles, fleet counters).
    Export {
        #[command(subcommand)]
        what: ExportWhat,
    },
    /// Console themes and mode: list, choose, check, import (`{home}/themes`).
    Theme {
        #[command(subcommand)]
        action: ThemeCmd,
    },
    /// Workspaces (one per organisation): list them, move a repository,
    /// rename, change a rule or the default (`{home}/workspaces.json`).
    Workspace {
        #[command(subcommand)]
        action: Option<WorkspaceCmd>,
    },
    /// Is there a newer harbard? Show what changed and install it: fetch the
    /// source checkout, rebuild the CLI, restart serve, rebuild the console
    /// and the app (docs/SPEC-UPDATES.md).
    Update {
        /// Check only. Exit 0 when up to date, 10 when an update is available, 1 when the check failed.
        #[arg(long)]
        check: bool,
        /// Install without asking.
        #[arg(long, short = 'y')]
        yes: bool,
        /// Follow this channel from now on: main (every change) or releases (published versions).
        #[arg(long, value_name = "CHANNEL")]
        channel: Option<String>,
        /// Print nothing; the exit code answers.
        #[arg(long)]
        quiet: bool,
        /// Start the update and return; the console and `harbard update` follow it.
        #[arg(long)]
        detach: bool,
        /// Internal: the detached process that performs run <ID>.
        #[arg(long, hide = true, value_name = "ID")]
        run: Option<String>,
    },
}

#[derive(Subcommand, Debug)]
pub enum WorkspaceCmd {
    /// Every workspace, its rule, and the repositories the history places in it.
    Ls,
    /// Which workspace a path (default: the current directory) is in, and why.
    Where { path: Option<String> },
    /// Move a repository to a workspace by hand; wins over the rule.
    Move { path: String, workspace: String },
    /// Give a repository moved by hand back to the rule.
    Reset { path: String },
    /// Rename a workspace (what the rail and the sidebar show).
    Rename { workspace: String, name: String },
    /// Set the rail tile's one or two characters; omit to use the first letter.
    Icon {
        workspace: String,
        icon: Option<String>,
    },
    /// Make a workspace claim every repository whose origin this owner holds.
    Claim { workspace: String, owner: String },
    /// Stop claiming an owner: its repositories go to the default.
    Unclaim { owner: String },
    /// Choose the workspace that takes whatever no rule claims.
    Default { workspace: String },
    /// Add a workspace.
    Add {
        id: String,
        name: String,
        /// An origin owner it claims; repeat for more.
        #[arg(long)]
        owner: Vec<String>,
    },
    /// Remove a workspace; its repositories go to the default.
    Remove { workspace: String },
}

#[derive(Subcommand, Debug)]
pub enum ThemeCmd {
    /// Every theme, the current one marked, and the mode.
    List,
    /// Choose a theme by name (built-in or `{home}/themes/<name>.toml`).
    Set { name: String },
    /// Choose the mode.
    Mode {
        // The Spanish spellings stay as hidden aliases for installed hooks.
        #[arg(value_parser = clap::builder::PossibleValuesParser::new([
            clap::builder::PossibleValue::new("light").alias("claro"),
            clap::builder::PossibleValue::new("dark").alias("oscuro"),
            clap::builder::PossibleValue::new("system").alias("sistema"),
        ]))]
        mode: String,
    },
    /// Check a theme file; exit 1 with the reason when it is refused.
    Check { file: std::path::PathBuf },
    /// Create a theme file from a `harbard-theme:v1:` share code.
    Import {
        code: String,
        /// Replace an existing theme of the same name.
        #[arg(long)]
        force: bool,
    },
    /// Print the themes folder (created, with an example, if missing).
    Path,
}

#[derive(Subcommand, Debug)]
pub enum DependencyCmd {
    /// Declare who owes a result and how the requester should continue.
    Open {
        #[arg(long)]
        to: String,
        #[arg(long)]
        condition: String,
        #[arg(long = "resume")]
        continuation: String,
        /// Expected response deadline; passing it never grants permission or resolves work.
        #[arg(long, value_parser = clap::value_parser!(u64).range(1..=604_800))]
        deadline_secs: Option<u64>,
    },
    /// Report a result as the named responsible session; the requester must verify it.
    Resolve {
        id: String,
        #[arg(long, value_parser = ["satisfied", "failed"])]
        outcome: String,
        #[arg(long)]
        evidence: String,
    },
    /// Cancel an owned dependency with an explicit reason.
    Cancel {
        id: String,
        #[arg(long)]
        reason: String,
    },
    /// Mark a terminal result as handled by its requester, after verification.
    Consume { id: String },
    /// Inspect a dependency and whether a CLI listener is currently attached.
    Show { id: String },
    /// List dependencies involving this session, or the complete local ledger.
    List {
        #[arg(long)]
        all: bool,
    },
}

#[derive(Args, Debug, Clone)]
pub struct LaneTakeArgs {
    #[arg(value_name = "LANE")]
    pub lane: String,
    /// What you hold it for, one line (shown to everyone waiting).
    #[arg(long = "for", value_name = "TEXT")]
    pub purpose: String,
    /// How long the turn lasts without a heartbeat: `90s`, `60m`, `2h`.
    #[arg(long, value_name = "DURATION", default_value = "60m")]
    pub ttl: String,
    /// Release automatically once every listed PR is merged or closed:
    /// `pr:2258,pr:2263`.
    #[arg(long = "release-on", value_name = "pr:N,...", value_delimiter = ',')]
    pub release_on: Vec<String>,
}

#[derive(Subcommand, Debug)]
pub enum LaneCmd {
    /// Take the lane, or join its queue when someone holds it.
    Take(LaneTakeArgs),
    /// Join the lane's FIFO queue (takes it at once when it is free).
    Queue(LaneTakeArgs),
    /// Owner, ttl left and queue of one lane, or of every held lane here.
    Show {
        #[arg(value_name = "LANE")]
        lane: Option<String>,
    },
    /// Let go (the next waiter takes over), or leave the queue.
    Release {
        #[arg(value_name = "LANE")]
        lane: String,
    },
}

#[derive(Subcommand, Debug)]
pub enum ExportWhat {
    /// Note → inject → ack cycles kept in `learning.jsonl`.
    Learning {
        /// How far back: `7d` (default), `24h`, `30m`, or seconds.
        #[arg(long, value_name = "SPEC", default_value = "7d")]
        since: String,
    },
}

#[derive(Subcommand, Debug)]
pub enum HubCmd {
    /// Generate a hub token, bind serve on the tailnet, rewrite launchd, print invite.
    Enable {
        /// TCP port for serve (default 7419).
        #[arg(long, default_value_t = 7419)]
        port: u16,
        /// Reuse `~/.harbard/hub.token` if present (default). Pass to force a new token.
        #[arg(long)]
        rotate: bool,
        /// Write launchd but do not bootstrap/kickstart.
        #[arg(long)]
        no_reload: bool,
    },
    /// Restore loopback-only serve (keeps hub.token on disk).
    Disable {
        #[arg(long, default_value_t = 7419)]
        port: u16,
        #[arg(long)]
        no_reload: bool,
    },
    /// Issue one device's own credential and print its `harbard pair '…'`
    /// line. The invite works once, for one device, within a day.
    Invite {
        #[arg(long, default_value_t = 7419)]
        port: u16,
        /// The device's name in Preferences › Pairing (BOREALIS, MacBook…).
        #[arg(long)]
        name: String,
        /// `workshop` (default: runs agents) or `display` (a display).
        /// Operator is never an invite; promote with `harbard hub role`.
        #[arg(long, default_value = "workshop")]
        role: String,
        /// A workspace the device is granted when it pairs; repeatable.
        /// Without it the device sees the default workspace only.
        #[arg(long = "workspace", value_name = "ID")]
        workspaces: Vec<String>,
    },
    /// Revoke one device at once, without touching the others. With no
    /// device of that name, or with `--invite`, cancel a pending invite.
    Revoke {
        /// Device id, id prefix, or name; or a pending invite's name or ref.
        device: String,
        /// Only a pending invite (a leaked pair line), never a device.
        #[arg(long)]
        invite: bool,
    },
    /// Change a device's role: operator, workshop or display.
    Role {
        /// Device id, id prefix, or name.
        device: String,
        role: String,
        /// Required to promote to Operator, which reads everything.
        #[arg(long)]
        yes: bool,
    },
    /// Grant a paired device the workspaces it may see; it never sees the
    /// others. With none, it sees the default workspace only. An Operator
    /// device is the operator's own machine and sees every workspace.
    Grant {
        /// Device id, id prefix, or name.
        device: String,
        /// Workspace ids (`harbard workspace ls`).
        workspaces: Vec<String>,
    },
    /// Show hub readiness: token, address, devices, launchd.
    Status,
    /// Replace the shared hub token (0600) and say what to restart. Leaves
    /// per-device credentials (`credentials.json`) untouched; never prints
    /// the token.
    Rotate,
}

#[derive(Subcommand, Debug)]
pub enum DeviceCmd {
    /// Show or create this machine's stable device id (`device.json`).
    Show,
    /// Register / heartbeat this machine against `HARBARD_HUB` (satellite card).
    Join {
        /// Friendly name on the fleet card (default: hostname short).
        #[arg(long)]
        name: Option<String>,
    },
    /// List devices known to the hub (requires HARBARD_HUB).
    Ls,
}

#[derive(Subcommand, Debug)]
pub enum HuginnAction {
    /// Print every component a view may use and how to send one.
    Catalog,
    /// Check a .huginn view against the catalog without sending it.
    Check {
        #[arg(value_name = "PATH")]
        path: std::path::PathBuf,
    },
}
