//! Workspaces: one hub, several organisations.
//!
//! A workspace is an organisation: a name, a rail tile and a rule that claims
//! repositories. Every repository belongs to exactly one:
//!
//! 1. a repository the operator moved by hand ([`Workspaces::repos`]) sits
//!    where it was moved;
//! 2. otherwise the owner of its `origin` remote decides
//!    (`git@github.com:<owner>/<repo>`): the workspace whose
//!    [`WorkspaceDef::owners`] names that owner claims it;
//! 3. anything else (no remote, an owner nobody claims, a path this machine
//!    cannot read, no repository at all) falls in [`Workspaces::default`].
//!
//! The assignment is computed when an event is read, never written into the
//! log: the bus stays append-only, existing history is placed by the same rule
//! as new traffic, and moving a repository moves its history with it.
//!
//! The registry lives in `workspaces.json` under the harbard home (not to be
//! confused with `workspace.json`, the operator's desk). Without that file the
//! hub has a single implicit workspace and nothing is scoped: that is what a
//! hub looks like before serve seeds it ([`ensure`]).
//!
//! **Delivery.** [`Gate`] is what a reader carries into
//! [`crate::Event::is_addressed_to_reader`]: a note, ask, warning or ack is
//! delivered only within the writer's workspace. The operator's own notes are
//! never gated: what the operator addresses, the operator meant, and the
//! console scopes its own targeting (mentions, channel readers) to the
//! workspace it is showing.

use crate::event::Event;
use crate::store::Store;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// The registry file under the harbard home.
pub const FILE: &str = "workspaces.json";
/// Wire version of [`Workspaces`].
pub const VERSION: u32 = 1;
/// Id of the one implicit workspace of an unseeded hub.
pub const SINGLE_ID: &str = "harbard";
/// Name of the one implicit workspace: what the sidebar head always said.
pub const SINGLE_NAME: &str = "harbard";
/// Id the operator's own workspace is seeded with.
pub const PERSONAL_ID: &str = "personal";
/// Name the operator's own workspace is seeded with.
pub const PERSONAL_NAME: &str = "Personal";
/// Longest workspace name.
pub const NAME_MAX_CHARS: usize = 40;
/// Longest rail-tile icon, in characters.
pub const ICON_MAX_CHARS: usize = 2;

// ---------------------------------------------------------------------------
// The registry
// ---------------------------------------------------------------------------

/// One organisation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceDef {
    /// Stable, lowercase, `[a-z0-9-]`; what devices are granted and repos
    /// are moved to. Never changes once created.
    pub id: String,
    /// What the rail tile and the sidebar head show; the operator renames it.
    pub name: String,
    /// One or two characters on the rail tile; empty means the name's first
    /// letter ([`WorkspaceDef::tile`]).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub icon: String,
    /// Remote owners this workspace claims, compared case-insensitively.
    #[serde(default)]
    pub owners: Vec<String>,
}

impl WorkspaceDef {
    /// The rail tile's text: the icon, or the name's first letter.
    pub fn tile(&self) -> String {
        if !self.icon.trim().is_empty() {
            return self.icon.trim().to_string();
        }
        self.name
            .chars()
            .find(|c| c.is_alphanumeric())
            .map(|c| c.to_uppercase().collect())
            .unwrap_or_else(|| "·".to_string())
    }

    fn claims(&self, owner: &str) -> bool {
        self.owners.iter().any(|o| o.eq_ignore_ascii_case(owner))
    }
}

/// `workspaces.json`: the workspaces, their rules, the hand-moved
/// repositories and the default.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Workspaces {
    #[serde(default = "version")]
    pub v: u32,
    /// Id of the workspace that takes whatever no rule claims.
    pub default: String,
    pub workspaces: Vec<WorkspaceDef>,
    /// Repositories moved by hand: repository identity (the primary
    /// checkout's path) → workspace id. Wins over the rule.
    #[serde(default)]
    pub repos: BTreeMap<String, String>,
}

fn version() -> u32 {
    VERSION
}

/// Why a registry change was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EditError {
    UnknownWorkspace(String),
    DuplicateId(String),
    InvalidId(String),
    InvalidName,
    InvalidIcon,
    /// The default workspace cannot be removed; make another the default first.
    RemovingDefault,
    /// The last workspace cannot be removed.
    LastWorkspace,
}

impl std::fmt::Display for EditError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EditError::UnknownWorkspace(id) => write!(f, "no workspace has the id \"{id}\""),
            EditError::DuplicateId(id) => write!(f, "a workspace already has the id \"{id}\""),
            EditError::InvalidId(id) => write!(
                f,
                "\"{id}\" is not a workspace id: use 1-32 of a-z, 0-9 and -, starting with a letter or digit"
            ),
            EditError::InvalidName => write!(
                f,
                "a workspace name holds 1 to {NAME_MAX_CHARS} visible characters"
            ),
            EditError::InvalidIcon => write!(
                f,
                "a workspace icon holds 1 or {ICON_MAX_CHARS} visible characters"
            ),
            EditError::RemovingDefault => write!(
                f,
                "this is the default workspace; make another one the default first"
            ),
            EditError::LastWorkspace => write!(f, "the hub keeps at least one workspace"),
        }
    }
}

impl std::error::Error for EditError {}

/// True for a usable workspace id.
pub fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 32
        && id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        && !id.starts_with('-')
}

/// A workspace id from free text: `Acme-42-Platform` → `acme-42-platform`.
pub fn slug(raw: &str) -> String {
    let mut out = String::new();
    for c in raw.chars() {
        let c = c.to_ascii_lowercase();
        if c.is_ascii_lowercase() || c.is_ascii_digit() {
            out.push(c);
        } else if !out.ends_with('-') && !out.is_empty() {
            out.push('-');
        }
        if out.len() >= 32 {
            break;
        }
    }
    let out = out.trim_end_matches('-').to_string();
    if out.is_empty() {
        "workspace".to_string()
    } else {
        out
    }
}

fn clean_name(raw: &str) -> Result<String, EditError> {
    let name = raw.split_whitespace().collect::<Vec<_>>().join(" ");
    if name.is_empty()
        || name.chars().count() > NAME_MAX_CHARS
        || name.chars().any(|c| c.is_control())
    {
        return Err(EditError::InvalidName);
    }
    Ok(name)
}

impl Workspaces {
    /// The registry of a hub with one workspace (used only by tests and as
    /// the shape [`Placement::single`] stands for).
    pub fn single() -> Self {
        Workspaces {
            v: VERSION,
            default: SINGLE_ID.to_string(),
            workspaces: vec![WorkspaceDef {
                id: SINGLE_ID.to_string(),
                name: SINGLE_NAME.to_string(),
                icon: String::new(),
                owners: Vec::new(),
            }],
            repos: BTreeMap::new(),
        }
    }

    pub fn path(store: &Store) -> PathBuf {
        store.root().join(FILE)
    }

    /// The registry on disk: `Ok(None)` when the hub was never seeded,
    /// `Err` when the file is there but unreadable.
    pub fn load(store: &Store) -> io::Result<Option<Self>> {
        match fs::read_to_string(Self::path(store)) {
            Ok(raw) => serde_json::from_str::<Workspaces>(&raw)
                .map(|w| Some(w.normalized()))
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string())),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// The same registry with a default that names a real workspace.
    fn normalized(mut self) -> Self {
        if self.workspaces.is_empty() {
            return Workspaces::single();
        }
        if !self.workspaces.iter().any(|w| w.id == self.default) {
            self.default = self.workspaces[0].id.clone();
        }
        self
    }

    pub fn get(&self, id: &str) -> Option<&WorkspaceDef> {
        self.workspaces.iter().find(|w| w.id == id)
    }

    fn get_mut(&mut self, id: &str) -> Result<&mut WorkspaceDef, EditError> {
        self.workspaces
            .iter_mut()
            .find(|w| w.id == id)
            .ok_or_else(|| EditError::UnknownWorkspace(id.to_string()))
    }

    fn require(&self, id: &str) -> Result<(), EditError> {
        self.get(id)
            .map(|_| ())
            .ok_or_else(|| EditError::UnknownWorkspace(id.to_string()))
    }

    /// Add a workspace. Its id must be new; owners may be empty.
    pub fn add(&mut self, id: &str, name: &str, owners: &[String]) -> Result<(), EditError> {
        if !valid_id(id) {
            return Err(EditError::InvalidId(id.to_string()));
        }
        if self.get(id).is_some() {
            return Err(EditError::DuplicateId(id.to_string()));
        }
        let name = clean_name(name)?;
        let mut def = WorkspaceDef {
            id: id.to_string(),
            name,
            icon: String::new(),
            owners: Vec::new(),
        };
        for owner in owners {
            push_owner(&mut def.owners, owner);
        }
        // An owner belongs to one workspace: the rule must stay a function.
        for other in &mut self.workspaces {
            other.owners.retain(|o| !def.claims(o));
        }
        self.workspaces.push(def);
        Ok(())
    }

    pub fn rename(&mut self, id: &str, name: &str) -> Result<(), EditError> {
        let name = clean_name(name)?;
        self.get_mut(id)?.name = name;
        Ok(())
    }

    /// Set the rail tile's text; an empty icon goes back to the first letter.
    pub fn set_icon(&mut self, id: &str, icon: &str) -> Result<(), EditError> {
        let icon = icon.trim();
        let n = icon.chars().count();
        if n > ICON_MAX_CHARS || icon.chars().any(|c| c.is_control() || c.is_whitespace()) {
            return Err(EditError::InvalidIcon);
        }
        self.get_mut(id)?.icon = icon.to_string();
        Ok(())
    }

    /// Make `id` claim repositories whose origin is owned by `owner`, taking
    /// the owner from whichever workspace held it.
    pub fn claim(&mut self, id: &str, owner: &str) -> Result<(), EditError> {
        self.require(id)?;
        let owner = owner.trim();
        if owner.is_empty() {
            return Err(EditError::InvalidName);
        }
        for w in &mut self.workspaces {
            w.owners.retain(|o| !o.eq_ignore_ascii_case(owner));
        }
        push_owner(&mut self.get_mut(id)?.owners, owner);
        Ok(())
    }

    /// Stop claiming `owner` anywhere: its repositories go to the default.
    pub fn unclaim(&mut self, owner: &str) -> bool {
        let mut changed = false;
        for w in &mut self.workspaces {
            let before = w.owners.len();
            w.owners.retain(|o| !o.eq_ignore_ascii_case(owner.trim()));
            changed |= w.owners.len() != before;
        }
        changed
    }

    pub fn set_default(&mut self, id: &str) -> Result<(), EditError> {
        self.require(id)?;
        self.default = id.to_string();
        Ok(())
    }

    /// Move a repository by hand. `repo` is its identity (the primary
    /// checkout's path); [`repository_root`] turns a worktree into it.
    pub fn pin(&mut self, repo: &str, id: &str) -> Result<(), EditError> {
        self.require(id)?;
        self.repos.insert(repo.to_string(), id.to_string());
        Ok(())
    }

    /// Give a hand-moved repository back to the rule. True when it was moved.
    pub fn unpin(&mut self, repo: &str) -> bool {
        self.repos.remove(repo).is_some()
    }

    /// Remove a workspace: its owners are claimed by nobody (so its
    /// repositories fall to the default) and repositories moved into it go
    /// back to the rule.
    pub fn remove(&mut self, id: &str) -> Result<WorkspaceDef, EditError> {
        self.require(id)?;
        if self.workspaces.len() == 1 {
            return Err(EditError::LastWorkspace);
        }
        if self.default == id {
            return Err(EditError::RemovingDefault);
        }
        self.repos.retain(|_, w| w != id);
        let at = self
            .workspaces
            .iter()
            .position(|w| w.id == id)
            .unwrap_or_default();
        Ok(self.workspaces.remove(at))
    }
}

fn push_owner(owners: &mut Vec<String>, owner: &str) {
    let owner = owner.trim();
    if !owner.is_empty() && !owners.iter().any(|o| o.eq_ignore_ascii_case(owner)) {
        owners.push(owner.to_string());
    }
}

// ---------------------------------------------------------------------------
// What a repository says about itself
// ---------------------------------------------------------------------------

/// The facts the rule reads off a checkout.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RepoFacts {
    /// The repository's identity: the primary checkout's path, also for a
    /// path inside a linked worktree. `None` when the path is not a checkout
    /// this machine can read.
    pub root: Option<String>,
    /// The owner of its `origin` remote.
    pub owner: Option<String>,
}

/// The `.git` of the checkout `path` is in: its own, or the nearest
/// ancestor's (a lead's working directory is often a subdirectory).
fn git_dir(path: &Path) -> Option<PathBuf> {
    path.ancestors().take(64).find_map(own_git_dir)
}

/// The `.git` directory of `repo` itself: a directory, or a `gitdir:` pointer.
fn own_git_dir(repo: &Path) -> Option<PathBuf> {
    let git = repo.join(".git");
    let meta = fs::metadata(&git).ok()?;
    if meta.is_dir() {
        return Some(git);
    }
    let contents = fs::read_to_string(&git).ok()?;
    let pointer = PathBuf::from(contents.trim().strip_prefix("gitdir:")?.trim());
    Some(if pointer.is_absolute() {
        pointer
    } else {
        repo.join(pointer)
    })
}

/// The common `.git` of a checkout (the primary's, for a linked worktree).
fn common_dir(repo: &Path) -> Option<PathBuf> {
    let dir = git_dir(repo)?;
    let common = fs::read_to_string(dir.join("commondir"))
        .ok()
        .map(|c| {
            let c = PathBuf::from(c.trim());
            if c.is_absolute() { c } else { dir.join(c) }
        })
        .unwrap_or(dir);
    Some(fs::canonicalize(&common).unwrap_or(common))
}

/// The repository identity of a checkout path: the primary checkout's path,
/// or the path itself when it is not a readable checkout.
pub fn repository_root(path: &str) -> String {
    read_facts(path).root.unwrap_or_else(|| path.to_string())
}

/// Read a checkout's identity and origin owner from its git files; no
/// process is spawned.
pub fn read_facts(path: &str) -> RepoFacts {
    let repo = Path::new(path);
    let Some(common) = common_dir(repo) else {
        return RepoFacts::default();
    };
    let root = if common.file_name().is_some_and(|n| n == ".git") {
        common.parent().map(|p| p.to_string_lossy().into_owned())
    } else {
        // A bare repository: its own directory is its identity.
        Some(common.to_string_lossy().into_owned())
    };
    let owner = fs::read_to_string(common.join("config"))
        .ok()
        .and_then(|config| origin_url(&config))
        .and_then(|url| owner_of_url(&url));
    RepoFacts { root, owner }
}

/// The `url` of `[remote "origin"]` in a git config file.
pub fn origin_url(config: &str) -> Option<String> {
    let mut inside = false;
    for line in config.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            let header: String = line.chars().filter(|c| !c.is_whitespace()).collect();
            inside = header == "[remote\"origin\"]";
            continue;
        }
        if !inside {
            continue;
        }
        if let Some((key, value)) = line.split_once('=')
            && key.trim().eq_ignore_ascii_case("url")
        {
            let value = value.trim().trim_matches('"').trim();
            if !value.is_empty() {
                return Some(value.to_string());
            }
        }
    }
    None
}

/// The owner a remote URL names: the first segment of its path.
///
/// `git@github.com:Owner/repo.git`, `ssh://git@host:22/Owner/repo`,
/// `https://user@host/Owner/repo` → `Owner`. Azure DevOps SSH remotes carry
/// a `v3/` before the organisation, which is skipped. A local path or a
/// `file://` URL has no owner.
pub fn owner_of_url(url: &str) -> Option<String> {
    let url = url.trim();
    let (host, path) = if let Some((scheme, rest)) = url.split_once("://") {
        if scheme.eq_ignore_ascii_case("file") {
            return None;
        }
        let (authority, path) = rest.split_once('/')?;
        let host = authority.rsplit('@').next().unwrap_or(authority);
        (host.split(':').next().unwrap_or(host), path)
    } else {
        // scp-like: [user@]host:path, with no slash before the colon.
        let (before, path) = url.split_once(':')?;
        if before.contains('/') || before.is_empty() {
            return None;
        }
        (before.rsplit('@').next().unwrap_or(before), path)
    };
    let mut segments = path.split('/').filter(|s| !s.is_empty());
    let mut first = segments.next()?;
    if first == "v3" && host.to_ascii_lowercase().ends_with("dev.azure.com") {
        first = segments.next()?;
    }
    let owner = first.trim_end_matches(".git");
    (!owner.is_empty() && owner != "." && owner != "..").then(|| owner.to_string())
}

// ---------------------------------------------------------------------------
// Placement: which workspace an event or a reader is in
// ---------------------------------------------------------------------------

type FactsFn = dyn Fn(&str) -> RepoFacts + Send + Sync;

/// Why a repository sits in its workspace.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reason {
    /// The operator moved it there.
    Moved,
    /// Its `origin` is owned by this owner, which the workspace claims.
    Owner(String),
    /// This hub cannot read it; the paired device it came from is granted
    /// that workspace.
    Device(String),
    /// Nothing claims it.
    Default,
}

impl std::fmt::Display for Reason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Reason::Moved => write!(f, "moved by hand"),
            Reason::Owner(o) => write!(f, "origin owner {o}"),
            Reason::Device(h) => write!(f, "paired device {h}"),
            Reason::Default => write!(f, "default workspace"),
        }
    }
}

/// Answers "which workspace is this in", with the repository facts read
/// once per path per process.
pub struct Placement {
    /// `None`: the hub has one workspace and nothing is scoped.
    set: Option<Workspaces>,
    /// The registry could not be read: the rule alone places repositories,
    /// one workspace per origin owner. Fails toward separation.
    owners_only: bool,
    facts: Box<FactsFn>,
    memo: Mutex<HashMap<String, String>>,
    /// Paired devices' hosts and the one workspace each is granted: an event
    /// from a path this hub cannot read is placed by its device.
    hosts: HashMap<String, String>,
}

impl std::fmt::Debug for Placement {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Placement")
            .field("set", &self.set)
            .field("owners_only", &self.owners_only)
            .field("hosts", &self.hosts)
            .finish_non_exhaustive()
    }
}

/// Id of the bucket for unplaced repositories when the registry is unreadable.
const FALLBACK_DEFAULT: &str = "default";

impl Placement {
    /// A hub with one workspace: every gate is open.
    pub fn single() -> Self {
        Placement {
            set: None,
            owners_only: false,
            facts: Box::new(read_facts),
            memo: Mutex::new(HashMap::new()),
            hosts: HashMap::new(),
        }
    }

    /// The hub's placement: the registry on disk, read with the real git
    /// files, and the paired devices' grants. An unseeded hub is
    /// [`Placement::single`].
    pub fn load(store: &Store) -> Self {
        match Workspaces::load(store) {
            Ok(Some(set)) => {
                let hosts = device_hosts(&crate::device::DeviceRegistry::load(store), &set);
                Self::with(Some(set), read_facts).with_device_hosts(hosts)
            }
            Ok(None) => Self::with(None, read_facts),
            Err(_) => Placement {
                set: None,
                owners_only: true,
                facts: Box::new(read_facts),
                memo: Mutex::new(HashMap::new()),
                hosts: HashMap::new(),
            },
        }
    }

    /// A placement over `set` with injected repository facts (tests).
    pub fn with(
        set: Option<Workspaces>,
        facts: impl Fn(&str) -> RepoFacts + Send + Sync + 'static,
    ) -> Self {
        Placement {
            set: set.map(Workspaces::normalized),
            owners_only: false,
            facts: Box::new(facts),
            memo: Mutex::new(HashMap::new()),
            hosts: HashMap::new(),
        }
    }

    /// Place events from `host` (a paired device) in `workspace` when their
    /// repository cannot be read here.
    pub fn with_device_hosts(mut self, hosts: HashMap<String, String>) -> Self {
        self.hosts = hosts;
        self
    }

    /// The registry, when there is one.
    pub fn registry(&self) -> Option<&Workspaces> {
        self.set.as_ref()
    }

    /// True when nothing is scoped: one workspace, or none on disk.
    pub fn is_single(&self) -> bool {
        !self.owners_only && self.set.as_ref().is_none_or(|s| s.workspaces.len() <= 1)
    }

    /// The id of the workspace whatever no rule claims falls in.
    pub fn default_id(&self) -> &str {
        match &self.set {
            Some(set) => &set.default,
            None if self.owners_only => FALLBACK_DEFAULT,
            None => SINGLE_ID,
        }
    }

    fn facts_of(&self, path: &str) -> RepoFacts {
        (self.facts)(path)
    }

    fn place_uncached(&self, repo: &str, host: Option<&str>) -> (String, Reason) {
        let facts = self.facts_of(repo);
        if self.owners_only {
            return match facts.owner {
                Some(o) => (
                    format!("owner:{}", o.to_ascii_lowercase()),
                    Reason::Owner(o),
                ),
                None => (FALLBACK_DEFAULT.to_string(), Reason::Default),
            };
        }
        let Some(set) = &self.set else {
            return (SINGLE_ID.to_string(), Reason::Default);
        };
        let pinned = set
            .repos
            .get(repo)
            .or_else(|| facts.root.as_deref().and_then(|root| set.repos.get(root)));
        if let Some(id) = pinned
            && set.get(id).is_some()
        {
            return (id.clone(), Reason::Moved);
        }
        if let Some(owner) = facts.owner.as_deref()
            && let Some(w) = set.workspaces.iter().find(|w| w.claims(owner))
        {
            return (w.id.clone(), Reason::Owner(owner.to_string()));
        }
        if facts.root.is_none()
            && let Some(h) = host
            && let Some(id) = self.hosts.get(h)
            && set.get(id).is_some()
        {
            return (id.clone(), Reason::Device(h.to_string()));
        }
        (set.default.clone(), Reason::Default)
    }

    /// Where `repo` is placed and why (`harbard workspace where`).
    pub fn explain(&self, repo: Option<&str>) -> (String, Reason) {
        match repo.filter(|r| !r.is_empty()) {
            Some(repo) if !self.is_single() => self.place_uncached(repo, None),
            _ => (self.default_id().to_string(), Reason::Default),
        }
    }

    /// The workspace of a repository (a checkout path, or `None` for a
    /// session outside any repository).
    pub fn of_repo(&self, repo: Option<&str>) -> String {
        self.of_repo_from(repo, None)
    }

    fn of_repo_from(&self, repo: Option<&str>, host: Option<&str>) -> String {
        if self.is_single() {
            return self.default_id().to_string();
        }
        let Some(repo) = repo.filter(|r| !r.is_empty()) else {
            if let Some(id) = host.and_then(|h| self.hosts.get(h))
                && self.set.as_ref().is_some_and(|s| s.get(id).is_some())
            {
                return id.clone();
            }
            return self.default_id().to_string();
        };
        let key = match host {
            Some(h) if self.hosts.contains_key(h) => format!("{h}\u{0}{repo}"),
            _ => repo.to_string(),
        };
        if let Some(hit) = self
            .memo
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(&key)
        {
            return hit.clone();
        }
        let placed = self.place_uncached(repo, host).0;
        self.memo
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(key, placed.clone());
        placed
    }

    /// The workspace an event was written in: its repository's.
    pub fn of_event(&self, event: &Event) -> String {
        self.of_repo_from(event.repository(), Some(event.host.as_str()))
    }

    /// The gate a reader in `repo` carries (its repository identity, or its
    /// checkout when it has no other).
    pub fn gate(&self, repo: Option<&str>) -> Gate<'_> {
        if self.is_single() {
            return Gate::Open;
        }
        Gate::Within {
            placement: self,
            workspace: self.of_repo(repo),
        }
    }

    /// The gate of a session seen on the bus ([`Gate::holds_session`]).
    pub fn gate_of_session(&self, view: &crate::query::SessionView) -> Gate<'_> {
        if self.is_single() {
            return Gate::Open;
        }
        let repo = view.repo_key.as_deref().or(view.repo.as_deref());
        Gate::Within {
            placement: self,
            workspace: self.of_repo_from(repo, Some(view.host.as_str())),
        }
    }

    /// The gate of the workspace `id` itself (serve's views and writes).
    pub fn gate_of(&self, id: &str) -> Gate<'_> {
        if self.is_single() {
            return Gate::Open;
        }
        Gate::Within {
            placement: self,
            workspace: id.to_string(),
        }
    }
}

/// The workspaces a paired device may see: every one for an Operator
/// device (the operator's own machine); otherwise its grants that still
/// exist, or the default workspace when it holds none.
pub fn device_grants(card: &crate::device::DeviceCard, set: &Workspaces) -> Vec<String> {
    if card.access == crate::device::Access::Operator {
        return set.workspaces.iter().map(|w| w.id.clone()).collect();
    }
    let granted: Vec<String> = card
        .workspaces
        .iter()
        .filter(|id| set.get(id).is_some())
        .cloned()
        .collect();
    if granted.is_empty() {
        vec![set.default.clone()]
    } else {
        granted
    }
}

/// Satellite hosts granted exactly one workspace: where their events go
/// when this hub cannot read the checkout they were written in.
fn device_hosts(reg: &crate::device::DeviceRegistry, set: &Workspaces) -> HashMap<String, String> {
    reg.devices
        .iter()
        .filter(|d| d.role == crate::device::DeviceRole::Satellite && d.revoked_ms.is_none())
        .filter_map(|d| match device_grants(d, set).as_slice() {
            [only] => Some((d.host.clone(), only.clone())),
            _ => None,
        })
        .collect()
}

/// What a reader carries into delivery: the boundary of its workspace.
#[derive(Debug, Clone)]
pub enum Gate<'a> {
    /// No boundary: a hub with one workspace, or a reader that is the
    /// operator's whole-hub view.
    Open,
    /// The reader is in `workspace`; `placement` places what it reads.
    Within {
        placement: &'a Placement,
        workspace: String,
    },
}

impl Gate<'_> {
    /// The open gate, borrowable for any lifetime.
    pub const OPEN: Gate<'static> = Gate::Open;

    /// True when `event` may be delivered to this reader as far as the
    /// workspace boundary goes. The operator's notes always pass; anything
    /// else must have been written in the reader's workspace.
    pub fn admits(&self, event: &Event) -> bool {
        match self {
            Gate::Open => true,
            Gate::Within {
                placement,
                workspace,
            } => event.is_operator() || placement.of_event(event) == *workspace,
        }
    }

    /// True when a session or checkout in `repo` is inside this boundary:
    /// the live sessions, intents and co-presence a reader is told about.
    pub fn holds(&self, repo: Option<&str>) -> bool {
        match self {
            Gate::Open => true,
            Gate::Within {
                placement,
                workspace,
            } => placement.of_repo(repo) == *workspace,
        }
    }

    /// [`Gate::holds`] for a session seen on the bus: a session from a
    /// paired device whose checkout this hub cannot read is placed by its
    /// device's grant, as its events are.
    pub fn holds_session(&self, view: &crate::query::SessionView) -> bool {
        match self {
            Gate::Open => true,
            Gate::Within {
                placement,
                workspace,
            } => {
                let repo = view.repo_key.as_deref().or(view.repo.as_deref());
                placement.of_repo_from(repo, Some(view.host.as_str())) == *workspace
            }
        }
    }

    /// The workspace this gate is for, `None` when open.
    pub fn workspace(&self) -> Option<&str> {
        match self {
            Gate::Open => None,
            Gate::Within { workspace, .. } => Some(workspace),
        }
    }
}

// ---------------------------------------------------------------------------
// Seeding: the first registry, from the history the hub already holds
// ---------------------------------------------------------------------------

/// The GitHub login the operator's `gh` is signed in as, from its config
/// file (no network, no process).
pub fn operator_login() -> Option<String> {
    let dir = std::env::var_os("GH_CONFIG_DIR")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("XDG_CONFIG_HOME").map(|x| PathBuf::from(x).join("gh")))
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config/gh")))?;
    let raw = fs::read_to_string(dir.join("hosts.yml")).ok()?;
    gh_login(&raw)
}

/// The `user:` of the `github.com:` block of a `gh` hosts file.
pub fn gh_login(hosts_yml: &str) -> Option<String> {
    let mut inside = false;
    for line in hosts_yml.lines() {
        if line.trim().is_empty() || line.trim_start().starts_with('#') {
            continue;
        }
        let indented = line.starts_with(' ') || line.starts_with('\t');
        if !indented {
            inside = line.trim_end() == "github.com:";
            continue;
        }
        if inside {
            let trimmed = line.trim();
            // Only the block's own key, not a nested `users:` entry.
            let depth = line.len() - line.trim_start().len();
            if depth <= 4
                && let Some(user) = trimmed.strip_prefix("user:")
            {
                let user = user.trim().trim_matches('"').trim_matches('\'');
                if !user.is_empty() {
                    return Some(user.to_string());
                }
            }
        }
    }
    None
}

/// The first registry for a hub whose history touched `repos`.
///
/// One workspace per origin owner among them. The owner the operator's `gh`
/// is signed in as becomes "Personal" and the default; the others keep
/// their owner's name until the operator renames them. Without a known
/// login the owner with the most repositories is the default. With fewer
/// than two owners the registry holds one workspace, and nothing is scoped.
pub fn seed<'a>(
    repos: impl IntoIterator<Item = &'a str>,
    login: Option<&str>,
    facts: &dyn Fn(&str) -> RepoFacts,
) -> Workspaces {
    // owner (as first spelled) → distinct repository identities
    let mut owners: BTreeMap<String, (String, BTreeSet<String>)> = BTreeMap::new();
    for repo in repos {
        let f = facts(repo);
        if let Some(owner) = f.owner {
            let entry = owners
                .entry(owner.to_ascii_lowercase())
                .or_insert_with(|| (owner.clone(), BTreeSet::new()));
            entry.1.insert(f.root.unwrap_or_else(|| repo.to_string()));
        }
    }
    let login_spelled = login.map(str::to_string);
    let login = login.map(str::to_ascii_lowercase);
    // Without a known login, the owner with the most repositories.
    let busiest = owners
        .iter()
        .max_by(|a, b| a.1.1.len().cmp(&b.1.1.len()).then(b.0.cmp(a.0)))
        .map(|(k, _)| k.clone());
    let default_owner = login.clone().or(busiest);
    let mut set = Workspaces {
        v: VERSION,
        default: String::new(),
        workspaces: Vec::new(),
        repos: BTreeMap::new(),
    };
    // The operator's own workspace first: it is the default and the first tile.
    if let (Some(login), Some(spelled)) = (&login, &login_spelled) {
        let spelled = owners
            .get(login)
            .map(|(s, _)| s.clone())
            .unwrap_or_else(|| spelled.clone());
        let _ = set.add(PERSONAL_ID, PERSONAL_NAME, &[spelled]);
        set.default = PERSONAL_ID.to_string();
    }
    for (key, (spelled, _)) in &owners {
        if login.as_deref() == Some(key.as_str()) {
            continue;
        }
        let mut id = slug(spelled);
        while set.get(&id).is_some() {
            id.push('x');
        }
        let _ = set.add(&id, spelled, std::slice::from_ref(spelled));
        if default_owner.as_deref() == Some(key.as_str()) && set.default.is_empty() {
            set.default = id;
        }
    }
    if set.workspaces.is_empty() {
        return Workspaces::single();
    }
    if set.default.is_empty() {
        set.default = set.workspaces[0].id.clone();
    }
    set
}

fn write_atomic(path: &Path, value: &Workspaces) -> io::Result<()> {
    static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let raw = serde_json::to_vec_pretty(value).map_err(io::Error::other)?;
    let n = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let tmp = path.with_extension(format!("json.tmp.{}.{n}", std::process::id()));
    let written = (|| {
        use std::io::Write;
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&tmp)?;
        file.write_all(&raw)?;
        file.sync_all()?;
        fs::rename(&tmp, path)
    })();
    if written.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    written
}

fn with_lock<T>(store: &Store, f: impl FnOnce() -> io::Result<T>) -> io::Result<T> {
    use fs2::FileExt;
    fs::create_dir_all(store.root())?;
    let lock = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(store.root().join(format!("{FILE}.lock")))?;
    lock.lock_exclusive()?;
    let out = f();
    let _ = FileExt::unlock(&lock);
    out
}

/// The registry, seeded from `repos` when the hub has none yet. Seeding is
/// done once, under a lock; an unreadable file is never overwritten.
pub fn ensure<'a>(
    store: &Store,
    repos: impl IntoIterator<Item = &'a str>,
    login: Option<&str>,
    facts: &dyn Fn(&str) -> RepoFacts,
) -> io::Result<(Workspaces, bool)> {
    with_lock(store, || {
        if let Some(existing) = Workspaces::load(store)? {
            return Ok((existing, false));
        }
        let seeded = seed(repos, login, facts);
        write_atomic(&Workspaces::path(store), &seeded)?;
        Ok((seeded, true))
    })
}

/// Change the registry under its lock. Refuses when the hub was never
/// seeded (there is nothing to change) or the file is unreadable.
pub fn update<T>(
    store: &Store,
    f: impl FnOnce(&mut Workspaces) -> Result<T, EditError>,
) -> io::Result<Result<T, EditError>> {
    with_lock(store, || {
        let mut set = Workspaces::load(store)?.unwrap_or_else(Workspaces::single);
        let before = set.clone();
        let out = f(&mut set);
        if out.is_ok() && set != before {
            write_atomic(&Workspaces::path(store), &set)?;
        }
        Ok(out)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::{EventKind, Role, SCHEMA_VERSION};

    pub(crate) fn facts_table(rows: &[(&str, &str, Option<&str>)]) -> impl Fn(&str) -> RepoFacts {
        let rows: Vec<(String, String, Option<String>)> = rows
            .iter()
            .map(|(p, r, o)| (p.to_string(), r.to_string(), o.map(str::to_string)))
            .collect();
        move |path: &str| {
            rows.iter()
                .find(|(p, _, _)| p == path)
                .map(|(_, root, owner)| RepoFacts {
                    root: Some(root.clone()),
                    owner: owner.clone(),
                })
                .unwrap_or_default()
        }
    }

    fn two() -> Workspaces {
        let mut set = Workspaces {
            v: VERSION,
            default: "me".into(),
            workspaces: Vec::new(),
            repos: BTreeMap::new(),
        };
        set.add("me", "Personal", &["me".into()]).unwrap();
        set.add("acme", "Acme", &["acme".into()]).unwrap();
        set
    }

    fn ev(repo: Option<&str>, operator: bool) -> Event {
        Event {
            v: SCHEMA_VERSION,
            id: "01J0000000000000000000000A".into(),
            ts: "2026-11-08T10:00:00Z".into(),
            event: EventKind::Note,
            session: "s".into(),
            harness: if operator { "human" } else { "claude-code" }.into(),
            host: "h".into(),
            cwd: repo.unwrap_or("").into(),
            repo: repo.map(str::to_string),
            repo_key: None,
            branch: None,
            role: if operator { Role::Operator } else { Role::Main },
            parent: None,
            text: "x".into(),
            to: None,
            kind: Default::default(),
            note_ref: None,
        }
    }

    #[test]
    fn owners_parse_from_every_remote_spelling() {
        for (url, owner) in [
            ("git@github.com:Acme/app.git", Some("Acme")),
            ("https://github.com/me/tool.git", Some("me")),
            ("https://user@github.com/me/tool", Some("me")),
            ("ssh://git@github.com:22/acme/app.git", Some("acme")),
            ("git@ssh.dev.azure.com:v3/acmeorg/Proj/app", Some("acmeorg")),
            (
                "https://acmeorg@dev.azure.com/acmeorg/Proj/_git/app",
                Some("acmeorg"),
            ),
            ("/srv/git/app.git", None),
            ("file:///srv/git/app.git", None),
            ("../sibling", None),
        ] {
            assert_eq!(owner_of_url(url).as_deref(), owner, "{url}");
        }
    }

    #[test]
    fn origin_is_read_from_its_own_section_only() {
        let config = "[core]\n\turl = nope\n[remote \"upstream\"]\n\turl = git@github.com:up/app.git\n[remote \"origin\"]\n\tfetch = +refs/heads/*\n\turl = git@github.com:Acme/app.git\n";
        assert_eq!(
            origin_url(config).as_deref(),
            Some("git@github.com:Acme/app.git")
        );
        assert_eq!(origin_url("[remote \"upstream\"]\nurl = x\n"), None);
    }

    #[test]
    fn real_checkouts_and_worktrees_resolve_to_the_primary_and_its_owner() {
        let home = crate::test_util::TempHome::new();
        let primary = home.path().join("app");
        fs::create_dir_all(primary.join(".git/worktrees/wt")).unwrap();
        fs::write(
            primary.join(".git/config"),
            "[remote \"origin\"]\n\turl = git@github.com:Acme/app.git\n",
        )
        .unwrap();
        fs::write(primary.join(".git/worktrees/wt/commondir"), "../..\n").unwrap();
        let wt = home.path().join("wt");
        fs::create_dir_all(&wt).unwrap();
        fs::write(
            wt.join(".git"),
            format!("gitdir: {}\n", primary.join(".git/worktrees/wt").display()),
        )
        .unwrap();
        let canon = fs::canonicalize(&primary).unwrap();
        for path in [&primary, &wt] {
            let f = read_facts(&path.to_string_lossy());
            assert_eq!(f.owner.as_deref(), Some("Acme"));
            assert_eq!(f.root.as_deref(), Some(&*canon.to_string_lossy()));
        }
        let sub = primary.join("src/deep");
        fs::create_dir_all(&sub).unwrap();
        assert_eq!(
            read_facts(&sub.to_string_lossy()).root.as_deref(),
            Some(&*canon.to_string_lossy()),
            "a subdirectory belongs to its checkout"
        );
        assert_eq!(read_facts("/definitely/not/here"), RepoFacts::default());
    }

    #[test]
    fn the_rule_places_by_owner_pins_win_and_the_rest_falls_to_the_default() {
        let mut set = two();
        set.pin("/w/moved", "me").unwrap();
        let p = Placement::with(
            Some(set),
            facts_table(&[
                ("/w/app", "/w/app", Some("ACME")),
                ("/w/app/.wt/x", "/w/app", Some("acme")),
                ("/w/tool", "/w/tool", Some("me")),
                ("/w/moved", "/w/moved", Some("acme")),
                ("/w/moved-wt", "/w/moved", Some("acme")),
                ("/w/oss", "/w/oss", Some("someone-else")),
                ("/w/local", "/w/local", None),
            ]),
        );
        assert!(!p.is_single());
        assert_eq!(p.of_repo(Some("/w/app")), "acme");
        assert_eq!(p.of_repo(Some("/w/app/.wt/x")), "acme");
        assert_eq!(p.of_repo(Some("/w/tool")), "me");
        assert_eq!(
            p.of_repo(Some("/w/moved")),
            "me",
            "a pin wins over the rule"
        );
        assert_eq!(
            p.of_repo(Some("/w/moved-wt")),
            "me",
            "and covers its worktrees"
        );
        assert_eq!(p.of_repo(Some("/w/oss")), "me");
        assert_eq!(p.of_repo(Some("/w/local")), "me");
        assert_eq!(p.of_repo(None), "me");
        assert_eq!(p.of_repo(Some("/not/on/this/machine")), "me");
    }

    #[test]
    fn device_hosts_place_events_this_hub_cannot_read() {
        let p =
            Placement::with(Some(two()), facts_table(&[])).with_device_hosts(HashMap::from([(
                "laptop~1".to_string(),
                "acme".to_string(),
            )]));
        let mut e = ev(Some("/home/dev/app"), false);
        assert_eq!(p.of_event(&e), "me");
        e.host = "laptop~1".into();
        assert_eq!(p.of_event(&e), "acme");
        e.repo = None;
        assert_eq!(p.of_event(&e), "acme");
    }

    #[test]
    fn a_single_workspace_and_an_unseeded_hub_open_every_gate() {
        let f = || facts_table(&[("/w/app", "/w/app", Some("acme"))]);
        for p in [
            Placement::with(None, f()),
            Placement::with(Some(Workspaces::single()), f()),
        ] {
            assert!(p.is_single());
            assert!(matches!(p.gate(Some("/w/app")), Gate::Open));
        }
    }

    #[test]
    fn the_gate_admits_its_own_workspace_and_the_operator_only() {
        let p = Placement::with(
            Some(two()),
            facts_table(&[
                ("/w/app", "/w/app", Some("acme")),
                ("/w/tool", "/w/tool", Some("me")),
            ]),
        );
        let in_acme = p.gate(Some("/w/app"));
        assert!(in_acme.admits(&ev(Some("/w/app"), false)));
        assert!(!in_acme.admits(&ev(Some("/w/tool"), false)));
        assert!(!in_acme.admits(&ev(None, false)), "no repo = the default");
        assert!(
            in_acme.admits(&ev(None, true)),
            "the operator is never gated"
        );
        assert!(in_acme.holds(Some("/w/app")));
        assert!(!in_acme.holds(Some("/w/tool")));
        let in_me = p.gate(None);
        assert!(in_me.admits(&ev(Some("/w/tool"), false)));
        assert!(!in_me.admits(&ev(Some("/w/app"), false)));
    }

    #[test]
    fn an_unreadable_registry_separates_by_owner_instead_of_opening() {
        let home = crate::test_util::TempHome::new();
        let store = Store::at(home.path());
        fs::write(home.path().join(FILE), "{ not json").unwrap();
        let p = Placement::load(&store);
        assert!(!p.is_single(), "a broken file must not erase the boundary");
    }

    #[test]
    fn seeding_names_the_operators_owner_personal_and_makes_it_the_default() {
        let facts = facts_table(&[
            ("/w/app", "/w/app", Some("Acme-Org")),
            ("/w/app2", "/w/app2", Some("acme-org")),
            ("/w/app2/.wt/a", "/w/app2", Some("acme-org")),
            ("/w/tool", "/w/tool", Some("Me")),
            ("/w/none", "/w/none", None),
        ]);
        let repos = ["/w/app", "/w/app2", "/w/app2/.wt/a", "/w/tool", "/w/none"];
        let set = seed(repos, Some("me"), &facts);
        assert_eq!(set.default, PERSONAL_ID);
        assert_eq!(set.workspaces.len(), 2);
        assert_eq!(set.workspaces[0].name, PERSONAL_NAME);
        assert_eq!(set.workspaces[0].owners, vec!["Me".to_string()]);
        let acme = set.get("acme-org").unwrap();
        assert_eq!(acme.name, "Acme-Org");
        assert_eq!(acme.tile(), "A");
        // The operator's own owner need not appear in the history yet.
        let set = seed(["/w/app"], Some("Me"), &facts);
        assert_eq!(set.default, PERSONAL_ID);
        assert_eq!(set.get(PERSONAL_ID).unwrap().owners, vec!["Me".to_string()]);
        assert_eq!(set.workspaces.len(), 2);
        // Without a login, the owner with most repositories is the default.
        let set = seed(repos, None, &facts);
        assert_eq!(set.default, "acme-org");
        // One owner only: one workspace, nothing scoped.
        let set = seed(["/w/tool"], Some("me"), &facts);
        assert_eq!(set.workspaces.len(), 1);
        assert!(Placement::with(Some(set), facts_table(&[])).is_single());
        assert_eq!(seed([], None, &facts), Workspaces::single());
    }

    #[test]
    fn gh_login_reads_the_github_block_only() {
        let raw = "gitlab.com:\n    user: other\ngithub.com:\n    git_protocol: https\n    users:\n        me: {}\n    user: me\n";
        assert_eq!(gh_login(raw).as_deref(), Some("me"));
        assert_eq!(gh_login("gitlab.com:\n    user: other\n"), None);
    }

    #[test]
    fn ensure_seeds_once_and_never_overwrites() {
        let home = crate::test_util::TempHome::new();
        let store = Store::at(home.path());
        let facts = facts_table(&[
            ("/w/app", "/w/app", Some("acme")),
            ("/w/tool", "/w/tool", Some("me")),
        ]);
        let (first, seeded) = ensure(&store, ["/w/app", "/w/tool"], Some("me"), &facts).unwrap();
        assert!(seeded);
        assert_eq!(first.workspaces.len(), 2);
        update(&store, |s| s.rename("acme", "Trabajo"))
            .unwrap()
            .unwrap();
        let (again, seeded) = ensure(&store, ["/w/other"], None, &facts).unwrap();
        assert!(!seeded);
        assert_eq!(again.get("acme").unwrap().name, "Trabajo");
        fs::write(home.path().join(FILE), "{ broken").unwrap();
        assert!(ensure(&store, ["/w/app"], None, &facts).is_err());
        assert_eq!(
            fs::read_to_string(home.path().join(FILE)).unwrap(),
            "{ broken"
        );
    }

    #[test]
    fn edits_keep_the_rule_a_function_and_the_default_real() {
        let mut set = two();
        set.claim("me", "ACME").unwrap();
        assert!(set.get("acme").unwrap().owners.is_empty());
        assert_eq!(set.get("me").unwrap().owners, vec!["me", "ACME"]);
        assert_eq!(set.remove("me"), Err(EditError::RemovingDefault));
        set.pin("/w/app", "acme").unwrap();
        set.remove("acme").unwrap();
        assert!(
            set.repos.is_empty(),
            "pins into a removed workspace go back to the rule"
        );
        assert_eq!(set.remove("me"), Err(EditError::LastWorkspace));
        assert_eq!(
            set.pin("/w/app", "ghost"),
            Err(EditError::UnknownWorkspace("ghost".into()))
        );
        assert_eq!(set.rename("me", "  "), Err(EditError::InvalidName));
        assert_eq!(set.set_icon("me", "abc"), Err(EditError::InvalidIcon));
        set.set_icon("me", "P").unwrap();
        assert_eq!(set.get("me").unwrap().tile(), "P");
        assert_eq!(slug("Acme-42-Platform"), "acme-42-platform");
        assert!(!valid_id("Bad Id"));
    }
}
#[cfg(test)]
mod boundary;
