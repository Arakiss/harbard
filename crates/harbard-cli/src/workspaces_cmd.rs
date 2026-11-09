//! `harbard workspace`: list, move, rename and re-rule the hub's workspaces
//! (`harbard_core::workspaces`).
//!
//! Every verb seeds the registry first when the hub has none, from the
//! repositories its history already holds, so the first `ls` shows what
//! serve would have seeded. Changes are written under the registry's lock;
//! serve and every hook read the file fresh, so a move takes effect at the
//! next prompt of every session without a restart.

use crate::cli::{Common, WorkspaceCmd};
use harbard_core::workspaces::{self, EditError, Placement, Workspaces};
use harbard_core::{Store, Window};
use serde_json::json;
use std::collections::{BTreeMap, BTreeSet};

fn refuse(message: impl std::fmt::Display) -> i32 {
    eprintln!("harbard workspace: {message}");
    1
}

/// Every repository identity the store's history names.
pub fn known_repos(store: &Store) -> BTreeSet<String> {
    store
        .read(Window::All)
        .events
        .iter()
        .filter_map(|e| e.repository())
        .map(str::to_string)
        .collect()
}

/// The registry, seeded from the history when the hub has none.
pub fn ensure(store: &Store) -> Result<(Workspaces, bool), String> {
    let repos = known_repos(store);
    let login = workspaces::operator_login();
    workspaces::ensure(
        store,
        repos.iter().map(String::as_str),
        login.as_deref(),
        &workspaces::read_facts,
    )
    .map_err(|e| format!("{} is unreadable: {e}", Workspaces::path(store).display()))
}

/// A path as the repository identity the registry keys moves by.
fn repo_identity(path: &str) -> String {
    let abs = std::path::Path::new(path);
    let abs = if abs.is_absolute() {
        abs.to_path_buf()
    } else {
        std::env::current_dir().unwrap_or_default().join(abs)
    };
    let abs = std::fs::canonicalize(&abs).unwrap_or(abs);
    let abs = abs.to_string_lossy().into_owned();
    // The checkout's top level, when `path` is inside one.
    let top = std::process::Command::new("git")
        .args(["-C", &abs, "rev-parse", "--show-toplevel"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or(abs);
    workspaces::repository_root(&top)
}

fn apply(
    store: &Store,
    common: &Common,
    what: &str,
    f: impl FnOnce(&mut Workspaces) -> Result<String, EditError>,
) -> i32 {
    match workspaces::update(store, f) {
        Err(e) => refuse(format!(
            "{} is unreadable: {e}",
            Workspaces::path(store).display()
        )),
        Ok(Err(e)) => refuse(e),
        Ok(Ok(line)) => {
            if common.json {
                println!("{}", json!({ "ok": true, "action": what, "message": line }));
            } else {
                println!("{line}");
            }
            0
        }
    }
}

pub fn cmd_workspace(store: &Store, common: &Common, action: Option<&WorkspaceCmd>) -> i32 {
    let (set, seeded) = match ensure(store) {
        Ok(v) => v,
        Err(e) => return refuse(e),
    };
    if seeded && !common.json {
        eprintln!(
            "seeded {} workspace(s) from this hub's history into {}",
            set.workspaces.len(),
            Workspaces::path(store).display()
        );
    }
    let Some(action) = action else {
        return list(store, common, &set);
    };
    match action {
        WorkspaceCmd::Ls => list(store, common, &set),
        WorkspaceCmd::Where { path } => {
            let path = path.clone().unwrap_or_else(|| ".".to_string());
            let repo = repo_identity(&path);
            let placement = Placement::load(store);
            let (id, why) = placement.explain(Some(&repo));
            let name = set.get(&id).map(|w| w.name.as_str()).unwrap_or(&id);
            if common.json {
                println!(
                    "{}",
                    json!({ "repo": repo, "workspace": id, "name": name, "reason": why.to_string() })
                );
            } else {
                println!("{repo}: {name} ({id}) — {why}");
            }
            0
        }
        WorkspaceCmd::Move { path, workspace } => {
            let repo = repo_identity(path);
            apply(store, common, "move", |s| {
                s.pin(&repo, workspace)?;
                let name = s.get(workspace).map(|w| w.name.clone()).unwrap_or_default();
                Ok(format!("{repo} → {name} ({workspace}), by hand"))
            })
        }
        WorkspaceCmd::Reset { path } => {
            let repo = repo_identity(path);
            apply(store, common, "reset", |s| {
                Ok(if s.unpin(&repo) {
                    format!("{repo} goes back to the rule")
                } else {
                    format!("{repo} was not moved by hand; nothing changed")
                })
            })
        }
        WorkspaceCmd::Rename { workspace, name } => apply(store, common, "rename", |s| {
            s.rename(workspace, name)?;
            Ok(format!(
                "{workspace} is now \"{}\"",
                s.get(workspace)
                    .map(|w| w.name.as_str())
                    .unwrap_or_default()
            ))
        }),
        WorkspaceCmd::Icon { workspace, icon } => apply(store, common, "icon", |s| {
            s.set_icon(workspace, icon.as_deref().unwrap_or(""))?;
            let tile = s.get(workspace).map(|w| w.tile()).unwrap_or_default();
            Ok(format!("{workspace} tile: {tile}"))
        }),
        WorkspaceCmd::Claim { workspace, owner } => apply(store, common, "claim", |s| {
            s.claim(workspace, owner)?;
            Ok(format!("repositories owned by {owner} → {workspace}"))
        }),
        WorkspaceCmd::Unclaim { owner } => apply(store, common, "unclaim", |s| {
            Ok(if s.unclaim(owner) {
                format!("{owner} is claimed by nobody; its repositories go to the default")
            } else {
                format!("no workspace claimed {owner}; nothing changed")
            })
        }),
        WorkspaceCmd::Default { workspace } => apply(store, common, "default", |s| {
            s.set_default(workspace)?;
            Ok(format!("{workspace} is the default workspace"))
        }),
        WorkspaceCmd::Add { id, name, owner } => apply(store, common, "add", |s| {
            s.add(id, name, owner)?;
            Ok(format!(
                "added {id} \"{}\"",
                s.get(id).map(|w| w.name.as_str()).unwrap_or_default()
            ))
        }),
        WorkspaceCmd::Remove { workspace } => apply(store, common, "remove", |s| {
            let gone = s.remove(workspace)?;
            Ok(format!(
                "removed {} \"{}\"; its repositories go to the default",
                gone.id, gone.name
            ))
        }),
    }
}

fn list(store: &Store, common: &Common, set: &Workspaces) -> i32 {
    let placement = Placement::load(store);
    let mut repos: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut seen = known_repos(store);
    seen.extend(set.repos.keys().cloned());
    for repo in &seen {
        repos
            .entry(placement.of_repo(Some(repo)))
            .or_default()
            .push(repo.clone());
    }
    if common.json {
        let rows: Vec<_> = set
            .workspaces
            .iter()
            .map(|w| {
                json!({
                    "id": w.id,
                    "name": w.name,
                    "tile": w.tile(),
                    "owners": w.owners,
                    "default": w.id == set.default,
                    "repos": repos.get(&w.id).cloned().unwrap_or_default(),
                    "moved": set.repos.iter().filter(|(_, id)| *id == &w.id).map(|(r, _)| r.clone()).collect::<Vec<_>>(),
                })
            })
            .collect();
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({ "default": set.default, "workspaces": rows }))
                .unwrap_or_default()
        );
        return 0;
    }
    for w in &set.workspaces {
        let default = if w.id == set.default {
            " · default"
        } else {
            ""
        };
        let owners = if w.owners.is_empty() {
            "claims no owner".to_string()
        } else {
            format!("claims {}", w.owners.join(", "))
        };
        println!("[{}] {} ({}){default} — {owners}", w.tile(), w.name, w.id);
        for repo in repos.get(&w.id).map(Vec::as_slice).unwrap_or_default() {
            let moved = if set.repos.contains_key(repo) {
                " (moved by hand)"
            } else {
                ""
            };
            println!("    {repo}{moved}");
        }
    }
    0
}
