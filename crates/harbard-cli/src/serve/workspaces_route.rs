//! `GET /api/v1/workspaces` and `POST /api/v1/workspaces/<action>`: the
//! hub's workspaces (`harbard_core::workspaces`) for the operator's console.
//!
//! The read lists every workspace, its rule, and where each repository the
//! history names is placed and why. The writes are the operator's desk
//! (loopback, no device): `add` a workspace (name, and at once its owners
//! and icon), `remove` one, `move` a
//! repository by hand, `reset` it to the rule, `rename` a workspace, set its
//! `icon`. They take the registry's lock,
//! like `harbard workspace`; every hook reads the file fresh, so a move holds
//! from the next prompt of every session on.

use super::fleet_routes::ApiError;
use harbard_core::workspaces::{self, EditError, Placement, Workspaces};
use harbard_core::{Event, Store};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

fn field<'a>(body: &'a Value, name: &'static str) -> Result<&'a str, ApiError> {
    body.get(name)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| ApiError::bad_request(name, format!("field `{name}` is required")))
}

/// `owners` of an `add`: absent, or a list of remote owners (an org or a
/// user); blanks are dropped.
fn owners_of(body: &Value) -> Result<Vec<String>, ApiError> {
    match body.get("owners") {
        None | Some(Value::Null) => Ok(Vec::new()),
        Some(Value::Array(list)) => {
            let mut owners = Vec::new();
            for v in list {
                let owner = v.as_str().map(str::trim).ok_or_else(|| {
                    ApiError::bad_request("owners", "field `owners` is a list of strings")
                })?;
                if !owner.is_empty() {
                    owners.push(owner.to_string());
                }
            }
            Ok(owners)
        }
        Some(_) => Err(ApiError::bad_request(
            "owners",
            "field `owners` is a list of strings",
        )),
    }
}

/// `GET /api/v1/workspaces`.
pub fn list_body(store: &Store, events: &[Event]) -> Value {
    list_with(&Placement::load(store), events)
}

/// [`list_body`] over a given placement (tests inject the repositories' facts).
fn list_with(placement: &Placement, events: &[Event]) -> Value {
    let set = placement
        .registry()
        .cloned()
        .unwrap_or_else(Workspaces::single);
    let mut repos: BTreeSet<String> = events
        .iter()
        .filter_map(|e| e.repository())
        .map(str::to_string)
        .collect();
    repos.extend(set.repos.keys().cloned());
    let placed: Vec<Value> = repos
        .iter()
        .map(|repo| {
            let (id, why) = placement.explain(Some(repo));
            json!({
                "repo": repo,
                "workspace": id,
                "moved": set.repos.contains_key(repo),
                "reason": why.to_string(),
            })
        })
        .collect();
    let rows: Vec<Value> = set
        .workspaces
        .iter()
        .map(|w| {
            json!({
                "id": w.id,
                "name": w.name,
                "tile": w.tile(),
                "icon": w.icon,
                "owners": w.owners,
                "default": w.id == set.default,
            })
        })
        .collect();
    json!({
        "v": 1,
        "single": placement.is_single(),
        "default": set.default,
        "workspaces": rows,
        "repos": placed,
        "owners": owners_body(placement, &set, &repos),
    })
}

/// The origin owners of the repositories the history names, the most
/// repositories first: for each, how many repositories a claim on it would
/// bring (one per repository, worktrees counted once; a repository moved by
/// hand stays where it was put, so it does not count), where they are now,
/// and which workspace claims it today.
fn owners_body(placement: &Placement, set: &Workspaces, repos: &BTreeSet<String>) -> Vec<Value> {
    // Lowercase owner -> (spelling first seen, workspace -> repository roots).
    let mut by: BTreeMap<String, (String, BTreeMap<String, BTreeSet<String>>)> = BTreeMap::new();
    for repo in repos {
        let facts = placement.facts(repo);
        let Some(owner) = facts.owner.filter(|o| !o.trim().is_empty()) else {
            continue;
        };
        let root = facts.root.unwrap_or_else(|| repo.clone());
        if set.repos.contains_key(repo) || set.repos.contains_key(&root) {
            continue;
        }
        let (workspace, _) = placement.explain(Some(repo));
        by.entry(owner.to_ascii_lowercase())
            .or_insert_with(|| (owner.clone(), BTreeMap::new()))
            .1
            .entry(workspace)
            .or_default()
            .insert(root);
    }
    let mut rows: Vec<(usize, String, Value)> = by
        .into_values()
        .map(|(owner, places)| {
            let total: BTreeSet<&String> = places.values().flatten().collect();
            let from: Vec<Value> = places
                .iter()
                .map(|(w, roots)| json!({ "workspace": w, "repos": roots.len() }))
                .collect();
            let claimed_by = set
                .workspaces
                .iter()
                .find(|w| w.owners.iter().any(|o| o.eq_ignore_ascii_case(&owner)))
                .map(|w| w.id.clone());
            let row = json!({
                "owner": owner,
                "repos": total.len(),
                "from": from,
                "claimed_by": claimed_by,
            });
            (total.len(), owner.to_ascii_lowercase(), row)
        })
        .collect();
    rows.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
    rows.into_iter().map(|(_, _, row)| row).collect()
}

fn edit_error(e: EditError) -> ApiError {
    let code = match e {
        EditError::UnknownWorkspace(_) => "unknown_workspace",
        EditError::InvalidName => "invalid_name",
        EditError::InvalidIcon => "invalid_icon",
        EditError::InvalidId(_) => "invalid_id",
        EditError::DuplicateId(_) => "duplicate_id",
        EditError::RemovingDefault => "removing_default",
        EditError::LastWorkspace => "last_workspace",
    };
    let status = match code {
        "unknown_workspace" => 404,
        "duplicate_id" => 409,
        _ => 422,
    };
    ApiError::new(status, code, e.to_string())
}

/// `POST /api/v1/workspaces/<action>`. `Ok(body)` is answered 200.
pub fn write(store: &Store, action: &str, body: &Value) -> Result<Value, ApiError> {
    let result = match action {
        "add" => {
            // The name is checked by the registry, so a blank one is
            // `invalid_name` like a rename's, not a missing field.
            let name = body
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            let id = body
                .get("id")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string);
            let owners = owners_of(body)?;
            let icon = body
                .get("icon")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string);
            // Name, owners and icon in one change: a refused icon leaves no
            // workspace behind, since nothing is written unless all hold.
            workspaces::update(store, |s| {
                if name.trim().is_empty() {
                    return Err(EditError::InvalidName);
                }
                let id = id.unwrap_or_else(|| s.free_id(&name));
                s.add(&id, &name, &owners)?;
                if let Some(icon) = &icon {
                    s.set_icon(&id, icon)?;
                }
                let w = s.get(&id);
                Ok(json!({
                    "ok": true,
                    "workspace": id,
                    "name": w.map(|w| w.name.clone()),
                    "tile": w.map(|w| w.tile()),
                    "owners": w.map(|w| w.owners.clone()),
                }))
            })
        }
        "remove" => {
            let workspace = field(body, "workspace")?.to_string();
            workspaces::update(store, |s| {
                let gone = s.remove(&workspace)?;
                Ok(json!({
                    "ok": true,
                    "workspace": gone.id,
                    "name": gone.name,
                    "default": s.default,
                }))
            })
        }
        "move" => {
            let repo = workspaces::repository_root(field(body, "repo")?);
            let workspace = field(body, "workspace")?.to_string();
            workspaces::update(store, |s| {
                s.pin(&repo, &workspace)?;
                Ok(json!({ "ok": true, "repo": repo, "workspace": workspace }))
            })
        }
        "reset" => {
            let repo = workspaces::repository_root(field(body, "repo")?);
            workspaces::update(store, |s| {
                let was = s.unpin(&repo);
                Ok(json!({ "ok": true, "repo": repo, "was_moved": was }))
            })
        }
        "rename" => {
            let workspace = field(body, "workspace")?.to_string();
            let name = field(body, "name")?.to_string();
            workspaces::update(store, |s| {
                s.rename(&workspace, &name)?;
                let name = s.get(&workspace).map(|w| w.name.clone());
                Ok(json!({ "ok": true, "workspace": workspace, "name": name }))
            })
        }
        "icon" => {
            let workspace = field(body, "workspace")?.to_string();
            let icon = body
                .get("icon")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            workspaces::update(store, |s| {
                s.set_icon(&workspace, &icon)?;
                let tile = s.get(&workspace).map(|w| w.tile());
                Ok(json!({ "ok": true, "workspace": workspace, "tile": tile }))
            })
        }
        _ => {
            return Err(ApiError::new(
                404,
                "unknown_action",
                format!("no workspaces action \"{action}\""),
            ));
        }
    };
    match result {
        Err(e) => Err(ApiError::new(
            500,
            "store_failed",
            format!("workspaces.json could not be changed: {e}"),
        )),
        Ok(Err(e)) => Err(edit_error(e)),
        Ok(Ok(v)) => Ok(v),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fresh hub with `harbard` (the default) and `acme`; `tag` keeps
    /// tests that run at once out of each other's directory.
    fn store_with_two(tag: &str) -> (std::path::PathBuf, Store) {
        let dir = std::env::temp_dir().join(format!(
            "harbard-ws-route-{tag}-{}-{}",
            std::process::id(),
            harbard_core::clock::now_ms()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let store = Store::at(&dir);
        let mut set = Workspaces::single();
        set.add("acme", "Acme", &["acme".into()]).unwrap();
        std::fs::write(
            Workspaces::path(&store),
            serde_json::to_string(&set).unwrap(),
        )
        .unwrap();
        (dir, store)
    }

    #[test]
    fn writes_move_reset_rename_and_refuse_with_a_reason() {
        let (dir, store) = store_with_two("edits");
        let moved = write(
            &store,
            "move",
            &json!({ "repo": "/nowhere/app", "workspace": "acme" }),
        )
        .unwrap();
        assert_eq!(moved["workspace"], "acme");
        let listed = list_body(&store, &[]);
        assert_eq!(listed["repos"][0]["repo"], "/nowhere/app");
        assert_eq!(listed["repos"][0]["workspace"], "acme");
        assert_eq!(listed["repos"][0]["moved"], true);
        let reset = write(&store, "reset", &json!({ "repo": "/nowhere/app" })).unwrap();
        assert_eq!(reset["was_moved"], true);
        write(
            &store,
            "rename",
            &json!({ "workspace": "acme", "name": "Trabajo" }),
        )
        .unwrap();
        assert_eq!(list_body(&store, &[])["workspaces"][1]["name"], "Trabajo");
        let e = write(
            &store,
            "move",
            &json!({ "repo": "/x", "workspace": "ghost" }),
        )
        .unwrap_err();
        assert_eq!((e.status, e.code), (404, "unknown_workspace"));
        let e = write(
            &store,
            "icon",
            &json!({ "workspace": "acme", "icon": "abc" }),
        )
        .unwrap_err();
        assert_eq!((e.status, e.code), (422, "invalid_icon"));
        let e = write(&store, "launch", &json!({})).unwrap_err();
        assert_eq!(e.status, 404);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn adds_with_an_id_from_the_name_and_removes_all_but_the_default() {
        let (dir, store) = store_with_two("add-remove");
        let added = write(&store, "add", &json!({ "name": "  Client work " })).unwrap();
        assert_eq!(added["workspace"], "client-work");
        assert_eq!(added["name"], "Client work");
        assert_eq!(added["tile"], "C");
        let again = write(
            &store,
            "add",
            &json!({ "name": "Client work", "owners": ["Globex", " ", "ACME"] }),
        )
        .unwrap();
        assert_eq!(again["workspace"], "client-work-2");
        assert_eq!(again["owners"], json!(["Globex", "ACME"]));
        let listed = list_body(&store, &[]);
        assert_eq!(listed["single"], false);
        let ids: Vec<&str> = listed["workspaces"]
            .as_array()
            .unwrap()
            .iter()
            .map(|w| w["id"].as_str().unwrap())
            .collect();
        assert_eq!(ids, vec!["harbard", "acme", "client-work", "client-work-2"]);
        assert_eq!(
            listed["workspaces"][1]["owners"],
            json!([]),
            "an owner belongs to one workspace"
        );
        let named = write(&store, "add", &json!({ "id": "ops", "name": "Ops" })).unwrap();
        assert_eq!(named["workspace"], "ops");

        let e = write(&store, "add", &json!({ "name": "   " })).unwrap_err();
        assert_eq!((e.status, e.code), (422, "invalid_name"));
        let e = write(&store, "add", &json!({})).unwrap_err();
        assert_eq!((e.status, e.code), (422, "invalid_name"));
        let e = write(&store, "add", &json!({ "id": "ops", "name": "Ops" })).unwrap_err();
        assert_eq!((e.status, e.code), (409, "duplicate_id"));
        let e = write(&store, "add", &json!({ "id": "Bad Id", "name": "x" })).unwrap_err();
        assert_eq!((e.status, e.code), (422, "invalid_id"));
        let e = write(&store, "add", &json!({ "name": "x", "owners": "acme" })).unwrap_err();
        assert_eq!((e.status, e.code), (400, "bad_request"));

        write(
            &store,
            "move",
            &json!({ "repo": "/nowhere/app", "workspace": "client-work" }),
        )
        .unwrap();
        let gone = write(&store, "remove", &json!({ "workspace": "client-work" })).unwrap();
        assert_eq!(gone["name"], "Client work");
        assert_eq!(gone["default"], "harbard");
        let listed = list_body(&store, &[]);
        assert_eq!(listed["workspaces"].as_array().unwrap().len(), 4);
        assert_eq!(
            listed["repos"],
            json!([]),
            "the repository moved into it is no longer moved"
        );
        let e = write(&store, "remove", &json!({ "workspace": "harbard" })).unwrap_err();
        assert_eq!((e.status, e.code), (422, "removing_default"));
        let e = write(&store, "remove", &json!({ "workspace": "ghost" })).unwrap_err();
        assert_eq!((e.status, e.code), (404, "unknown_workspace"));
        let e = write(&store, "remove", &json!({})).unwrap_err();
        assert_eq!(e.status, 400);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn an_icon_comes_with_the_add_or_nothing_is_added() {
        let (dir, store) = store_with_two("add-icon");
        let added = write(&store, "add", &json!({ "name": "Ops", "icon": "OP" })).unwrap();
        assert_eq!(added["tile"], "OP");
        let e = write(&store, "add", &json!({ "name": "Lab", "icon": "abc" })).unwrap_err();
        assert_eq!((e.status, e.code), (422, "invalid_icon"));
        let ids: Vec<String> = list_body(&store, &[])["workspaces"]
            .as_array()
            .unwrap()
            .iter()
            .map(|w| w["id"].as_str().unwrap().to_string())
            .collect();
        assert_eq!(ids, vec!["harbard", "acme", "ops"], "no half-made \"lab\"");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn owners_count_repositories_once_and_say_where_they_are() {
        use harbard_core::workspaces::RepoFacts;
        let mut set = Workspaces::single();
        set.add("personal", "Personal", &["Arakiss".into()])
            .unwrap();
        set.default = "personal".into();
        set.add("acme", "Acme", &["acme".into()]).unwrap();
        set.pin("/w/moved", "acme").unwrap();
        let facts = |path: &str| {
            let (root, owner) = match path {
                "/w/app" | "/w/app-wt" => ("/w/app", Some("Arakiss")),
                "/w/lib" => ("/w/lib", Some("arakiss")),
                "/w/site" => ("/w/site", Some("acme")),
                "/w/moved" => ("/w/moved", Some("Arakiss")),
                "/w/tool" => ("/w/tool", Some("globex")),
                _ => return RepoFacts::default(),
            };
            RepoFacts {
                root: Some(root.to_string()),
                owner: owner.map(str::to_string),
            }
        };
        let placement = Placement::with(Some(set.clone()), facts);
        let repos: BTreeSet<String> = [
            "/w/app",
            "/w/app-wt",
            "/w/lib",
            "/w/site",
            "/w/moved",
            "/w/tool",
            "/w/none",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        let rows = owners_body(&placement, &set, &repos);
        let owners: Vec<&str> = rows.iter().map(|r| r["owner"].as_str().unwrap()).collect();
        assert_eq!(
            owners,
            vec!["Arakiss", "acme", "globex"],
            "most repositories first"
        );
        assert_eq!(
            rows[0]["repos"], 2,
            "a worktree is its repository; a moved one stays"
        );
        assert_eq!(
            rows[0]["from"],
            json!([{ "workspace": "personal", "repos": 2 }])
        );
        assert_eq!(rows[0]["claimed_by"], "personal");
        assert_eq!(
            rows[1]["from"],
            json!([{ "workspace": "acme", "repos": 1 }])
        );
        assert_eq!(rows[2]["claimed_by"], Value::Null);
        assert_eq!(
            rows[2]["from"],
            json!([{ "workspace": "personal", "repos": 1 }])
        );
    }
}
