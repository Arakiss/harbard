//! What a team pass needs from the file system, kept between passes.
//!
//! A pass used to list every project and session under the Claude projects
//! root and stat every file of every `subagents` directory, several times
//! over, whether or not anything had moved: thousands of system calls per
//! read on a machine with a few months of sessions. This keeps the listings
//! and the per-file stats and redoes only what a change can have touched:
//!
//! - a directory is listed again when its own stamp (modification time and
//!   size) moves, which every entry added, removed or renamed in it does;
//! - a session inside the window is stat'ed file by file on every pass, as
//!   before, so what an agent appends shows on the next read;
//! - a session outside the window keeps its last stats while its lead
//!   transcript and its `subagents` directory keep their stamps. A lead that
//!   writes again, or a new agent file, brings it back on the next pass. An
//!   append to an old agent file that moves neither is seen when the
//!   session is stat'ed again, at most [`COLD_RESCAN`] later.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// How long a session outside the window keeps its stats when nothing
/// visible moved in it.
pub const COLD_RESCAN: Duration = Duration::from_secs(10);

/// What tells a file or directory changed: modification time and size.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Stamp {
    modified: SystemTime,
    len: u64,
}

impl Stamp {
    fn of(meta: &fs::Metadata) -> Option<Stamp> {
        Some(Stamp {
            modified: meta.modified().ok()?,
            len: meta.len(),
        })
    }

    fn ms(self) -> Option<u64> {
        Some(self.modified.duration_since(UNIX_EPOCH).ok()?.as_millis() as u64)
    }
}

/// Milliseconds since the epoch of a stat, as `team::mtime_ms` reads it.
fn ms_of(meta: &fs::Metadata) -> Option<u64> {
    Some(
        meta.modified()
            .ok()?
            .duration_since(UNIX_EPOCH)
            .ok()?
            .as_millis() as u64,
    )
}

/// One `<project>/<session>/subagents` directory found by a pass.
pub(crate) struct SessionDir {
    pub project: String,
    pub session: String,
    pub lead: PathBuf,
    pub subagents: PathBuf,
    /// The `subagents` directory's stamp when the pass found it.
    pub(crate) stamp: Option<Stamp>,
}

/// An entry of a listed directory, typed by the listing itself (no stat).
#[derive(Debug, Clone)]
struct Listed {
    name: String,
    /// A regular file never holds a directory: it needs no stat while the
    /// listing holds. A directory, a link or an unknown type is stat'ed.
    plain_file: bool,
}

#[derive(Debug, Default)]
struct Project {
    stamp: Option<Stamp>,
    sessions: Vec<Listed>,
}

/// One agent file of a `subagents` directory: `(id, transcript, meta)` and
/// the modification times a pass reads from them.
#[derive(Debug, Clone)]
pub(crate) struct AgentStat {
    pub id: String,
    pub path: PathBuf,
    pub meta_path: PathBuf,
    pub mtime: Option<u64>,
    pub meta_mtime: Option<u64>,
}

/// A `subagents` directory, stat'ed file by file.
#[derive(Debug)]
pub(crate) struct SubScan {
    dir: Option<Stamp>,
    lead: Option<Stamp>,
    at: Instant,
    /// Agents in id order, as `team::agent_files` lists them.
    pub agents: Vec<AgentStat>,
    /// The modification time of every regular file in the directory.
    files: Vec<u64>,
}

impl SubScan {
    /// Newest write of the lead or any agent file.
    pub fn last_write(&self, lead_ms: Option<u64>) -> u64 {
        self.agents
            .iter()
            .filter_map(|a| a.mtime.max(a.meta_mtime))
            .chain(lead_ms)
            .max()
            .unwrap_or(0)
    }

    /// Files written since `since`, for the format alarm.
    pub fn recent(&self, since: u64) -> usize {
        self.files.iter().filter(|m| **m >= since).count()
    }

    fn read(dir: &Path, dir_stamp: Option<Stamp>, lead: Option<Stamp>) -> SubScan {
        let mut entries: Vec<(String, Option<fs::Metadata>)> = Vec::new();
        if let Ok(it) = fs::read_dir(dir) {
            for e in it.flatten() {
                let name = e.file_name().to_string_lossy().into_owned();
                // Followed, as `Path::is_file` and `fs::metadata` follow.
                entries.push((name, fs::metadata(e.path()).ok()));
            }
        }
        let by_name: HashMap<&str, &fs::Metadata> = entries
            .iter()
            .filter_map(|(n, m)| Some((n.as_str(), m.as_ref()?)))
            .collect();
        let files = entries
            .iter()
            .filter_map(|(_, m)| m.as_ref())
            .filter(|m| m.is_file())
            .filter_map(ms_of)
            .collect();
        let mut agents: Vec<AgentStat> = entries
            .iter()
            .filter_map(|(name, meta)| {
                let id = name.strip_prefix("agent-")?.strip_suffix(".jsonl")?;
                let meta_name = format!("agent-{id}.meta.json");
                Some(AgentStat {
                    id: id.to_string(),
                    path: dir.join(name),
                    meta_path: dir.join(&meta_name),
                    mtime: meta.as_ref().and_then(ms_of),
                    meta_mtime: by_name.get(meta_name.as_str()).and_then(|m| ms_of(m)),
                })
            })
            .collect();
        agents.sort_by(|a, b| (&a.id, &a.path, &a.meta_path).cmp(&(&b.id, &b.path, &b.meta_path)));
        SubScan {
            dir: dir_stamp,
            lead,
            at: Instant::now(),
            agents,
            files,
        }
    }
}

/// The listings and stats of the previous passes.
#[derive(Debug)]
pub(crate) struct ScanCache {
    root: Option<Stamp>,
    /// Root entries in listing order.
    listed: Vec<Listed>,
    projects: HashMap<String, Project>,
    subagents: HashMap<PathBuf, SubScan>,
    pub cold_rescan: Duration,
}

impl Default for ScanCache {
    fn default() -> Self {
        ScanCache {
            root: None,
            listed: Vec::new(),
            projects: HashMap::new(),
            subagents: HashMap::new(),
            cold_rescan: COLD_RESCAN,
        }
    }
}

fn list(dir: &Path) -> Option<Vec<Listed>> {
    let it = fs::read_dir(dir).ok()?;
    Some(
        it.flatten()
            .map(|e| Listed {
                name: e.file_name().to_string_lossy().into_owned(),
                plain_file: e.file_type().is_ok_and(|t| t.is_file()),
            })
            .collect(),
    )
}

impl ScanCache {
    /// Every `<project>/<session>/subagents` directory under `root`, in
    /// listing order. `readable` is the caller's probe of the root.
    pub fn session_dirs(&mut self, root: &Path, readable: bool) -> Vec<SessionDir> {
        let mut out = Vec::new();
        if !readable {
            return out;
        }
        let root_stamp = fs::metadata(root).ok().as_ref().and_then(Stamp::of);
        if root_stamp.is_none() || root_stamp != self.root {
            match list(root) {
                Some(l) => {
                    self.listed = l;
                    self.root = root_stamp;
                }
                None => {
                    self.listed.clear();
                    self.root = None;
                }
            }
            let names: std::collections::HashSet<&str> =
                self.listed.iter().map(|l| l.name.as_str()).collect();
            self.projects.retain(|n, _| names.contains(n.as_str()));
        }
        for entry in &self.listed {
            if entry.plain_file {
                continue;
            }
            let ppath = root.join(&entry.name);
            let Ok(meta) = fs::metadata(&ppath) else {
                continue;
            };
            if !meta.is_dir() {
                continue;
            }
            let stamp = Stamp::of(&meta);
            let project = self.projects.entry(entry.name.clone()).or_default();
            if stamp.is_none() || stamp != project.stamp {
                match list(&ppath) {
                    Some(l) => {
                        project.sessions = l;
                        project.stamp = stamp;
                    }
                    None => {
                        project.sessions.clear();
                        project.stamp = None;
                    }
                }
            }
            for session in &project.sessions {
                if session.plain_file {
                    continue;
                }
                let subagents = ppath.join(&session.name).join("subagents");
                let Ok(sub) = fs::metadata(&subagents) else {
                    continue;
                };
                if !sub.is_dir() {
                    continue;
                }
                out.push(SessionDir {
                    project: entry.name.clone(),
                    lead: ppath.join(format!("{}.jsonl", session.name)),
                    session: session.name.clone(),
                    subagents,
                    stamp: Stamp::of(&sub),
                });
            }
        }
        out
    }

    /// The stats of one `subagents` directory. Kept from an earlier pass
    /// only while the session is outside the window (`since`) and neither
    /// its lead nor its directory moved, for at most `cold_rescan`.
    pub fn subagents(
        &mut self,
        dir: &SessionDir,
        lead: Option<&fs::Metadata>,
        since: u64,
    ) -> &SubScan {
        let lead_stamp = lead.and_then(Stamp::of);
        let lead_ms = lead_stamp.and_then(Stamp::ms);
        let fresh = |s: &SubScan| {
            s.dir.is_some()
                && s.dir == dir.stamp
                && s.lead == lead_stamp
                && s.last_write(lead_ms) < since
                && s.at.elapsed() < self.cold_rescan
        };
        let keep = self.subagents.get(&dir.subagents).is_some_and(fresh);
        if !keep {
            let scan = SubScan::read(&dir.subagents, dir.stamp, lead_stamp);
            self.subagents.insert(dir.subagents.clone(), scan);
        }
        &self.subagents[&dir.subagents]
    }

    /// Forget the directories the last pass did not find.
    pub fn retain(&mut self, found: &std::collections::HashSet<PathBuf>) {
        self.subagents.retain(|p, _| found.contains(p));
    }
}
