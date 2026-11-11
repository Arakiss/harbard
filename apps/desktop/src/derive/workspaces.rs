//! The hub's workspaces as the operator edits them from the app: creating
//! one from a name, removing one, and who the workspace menu offers to
//! switch to. serve picks the id and refuses what the registry forbids
//! (`serve/workspaces_route.rs`); this holds what the dialogs say before
//! and after sending.

use super::workspace::Words;
use crate::model::WorkspaceTile;
use crate::model::workspaces::{KnownOwner, WorkspaceEntry, WorkspacesList};
use gpui::SharedString;

/// The registry's limit on a name, in characters.
pub const NAME_MAX: usize = harbard_core::workspaces::NAME_MAX_CHARS;

/// The name as the hub keeps it: runs of spaces collapsed, trimmed.
pub fn clean_name(raw: &str) -> String {
    raw.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Whether the create dialog may send `raw`.
pub fn name_ok(raw: &str) -> bool {
    let name = clean_name(raw);
    !name.is_empty() && name.chars().count() <= NAME_MAX
}

/// The line under the name field once a submit was tried with nothing in it.
pub fn name_problem(raw: &str, tried: bool, w: &impl Words) -> Option<SharedString> {
    (tried && clean_name(raw).is_empty()).then(|| w.text("shell.newWorkspaceNameEmpty", &[]))
}

// ---------------------------------------------------------------------------
// Adding a workspace: three steps in one sheet
// ---------------------------------------------------------------------------

/// Where the "Add a workspace" sheet is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    /// Its name and tile letters, with the tile as the rail will show it.
    Name,
    /// Whose repositories live here (skippable).
    Owners,
    /// Created: the tile, and what moved.
    Ready,
}

impl Step {
    pub const ALL: [Step; 3] = [Step::Name, Step::Owners, Step::Ready];

    /// What Continue (or Enter) leads to; creating happens on the way to
    /// `Ready`, so the name must hold to leave `Name`.
    pub fn next(self, name: &str) -> Option<Step> {
        match self {
            Step::Name if name_ok(name) => Some(Step::Owners),
            Step::Name => None,
            Step::Owners => Some(Step::Ready),
            Step::Ready => None,
        }
    }

    /// What Back leads to. Once created there is no going back.
    pub fn back(self) -> Option<Step> {
        match self {
            Step::Owners => Some(Step::Name),
            _ => None,
        }
    }

    pub fn title_key(self) -> &'static str {
        match self {
            Step::Name => "shell.nwStepName",
            Step::Owners => "shell.nwStepOwners",
            Step::Ready => "shell.nwStepReady",
        }
    }
}

/// The id serve will give a workspace called `name`, so the preview draws
/// the very tile the rail will show.
pub fn predicted_id(list: &WorkspacesList, name: &str) -> String {
    let mut set = harbard_core::workspaces::Workspaces::single();
    set.workspaces = list
        .workspaces
        .iter()
        .map(|w| harbard_core::workspaces::WorkspaceDef {
            id: w.id.clone(),
            name: w.name.clone(),
            icon: String::new(),
            owners: Vec::new(),
        })
        .collect();
    if set.workspaces.is_empty() {
        // A hub never seeded: serve adds next to the one it stands for.
        set = harbard_core::workspaces::Workspaces::single();
    }
    set.free_id(&clean_name(name))
}

/// A workspace's name by id, the id itself when the list has no such one.
fn workspace_name(list: &WorkspacesList, id: &str) -> String {
    list.workspaces
        .iter()
        .find(|w| w.id == id)
        .map_or_else(|| id.to_string(), |w| w.name.clone())
}

/// "Personal", "Personal and Acme", "Personal, Acme and Ops".
pub fn join_names(names: &[String], w: &impl Words) -> String {
    match names {
        [] => String::new(),
        [one] => one.clone(),
        [rest @ .., last] => w
            .text("shell.listAnd", &[("a", &rest.join(", ")), ("b", last)])
            .to_string(),
    }
}

/// An owner row's second line: its repositories and where they are now.
pub fn owner_meta(list: &WorkspacesList, owner: &KnownOwner, w: &impl Words) -> SharedString {
    let places: Vec<String> = owner
        .from
        .iter()
        .map(|p| workspace_name(list, &p.workspace))
        .collect();
    let at = join_names(&places, w);
    if owner.repos == 1 {
        w.text("shell.ownerReposOne", &[("where", &at)])
    } else {
        w.text(
            "shell.ownerReposMany",
            &[("count", &owner.repos.to_string()), ("where", &at)],
        )
    }
}

/// What picking `selected` owners moves: how many repositories, and from
/// which workspaces (by name, in the hub's order).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Move {
    pub repos: u64,
    pub from: Vec<String>,
}

pub fn move_of(list: &WorkspacesList, selected: &[String]) -> Option<Move> {
    let picked: Vec<&KnownOwner> = list
        .owners
        .iter()
        .filter(|o| selected.iter().any(|s| s.eq_ignore_ascii_case(&o.owner)))
        .collect();
    let repos: u64 = picked.iter().map(|o| o.repos).sum();
    if repos == 0 {
        return None;
    }
    let from: Vec<String> = list
        .workspaces
        .iter()
        .filter(|w| {
            picked
                .iter()
                .any(|o| o.from.iter().any(|p| p.workspace == w.id && p.repos > 0))
        })
        .map(|w| w.name.clone())
        .collect();
    Some(Move { repos, from })
}

/// "4 repositories will move here from Personal." (`done`: "…moved here…");
/// without a move, that it starts empty.
pub fn move_summary(m: Option<&Move>, done: bool, w: &impl Words) -> SharedString {
    let Some(m) = m else {
        return w.text("shell.nwStartsEmpty", &[]);
    };
    let from = join_names(&m.from, w);
    let key = match (done, m.repos == 1) {
        (false, true) => "shell.nwMoveOne",
        (false, false) => "shell.nwMoveMany",
        (true, true) => "shell.nwMovedOne",
        (true, false) => "shell.nwMovedMany",
    };
    w.text(key, &[("count", &m.repos.to_string()), ("from", &from)])
}

/// Pick or unpick `owner`.
pub fn toggle_owner(selected: &mut Vec<String>, owner: &str) {
    if let Some(at) = selected.iter().position(|s| s.eq_ignore_ascii_case(owner)) {
        selected.remove(at);
    } else {
        selected.push(owner.to_string());
    }
}

/// serve's refusal of a workspaces write in the operator's words; a code
/// this build does not know keeps serve's message.
pub fn refusal(code: Option<&str>, message: &str, w: &impl Words) -> SharedString {
    match code {
        Some("invalid_name") => w.text(
            "shell.workspaceNameInvalid",
            &[("max", &NAME_MAX.to_string())],
        ),
        Some("removing_default") => w.text("shell.removeDefaultRefused", &[]),
        Some("last_workspace") => w.text("shell.removeLastRefused", &[]),
        // A bus older than the app: it answers the action it does not know.
        Some("unknown_action") => w.text("shell.workspacesBusTooOld", &[]),
        _ => message.to_string().into(),
    }
}

/// One row of the workspace menu.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceRow {
    pub id: String,
    pub label: SharedString,
    /// The one on screen: checked, and a click on it only closes the menu.
    pub current: bool,
}

/// The workspace menu's list: every workspace, a single one too, in the
/// hub's order (the rail's order), each with who is working there.
pub fn workspace_rows(tiles: &[WorkspaceTile], w: &impl Words) -> Vec<WorkspaceRow> {
    tiles
        .iter()
        .map(|t| WorkspaceRow {
            id: t.id.clone(),
            label: if t.working > 0 {
                w.text(
                    "shell.tileWorking",
                    &[("name", &t.name), ("count", &t.working.to_string())],
                )
            } else {
                t.name.clone().into()
            },
            current: t.current,
        })
        .collect()
}

/// Whether the settings offer to remove `w`: never the default, which
/// takes what no rule claims, and never the last one.
pub fn removable(list: &WorkspacesList, w: &WorkspaceEntry) -> bool {
    !w.default && w.id != list.default && list.workspaces.len() > 1
}

/// The workspace the default is, by name, for the remove dialog's sentence.
pub fn default_name(list: &WorkspacesList) -> String {
    list.workspaces
        .iter()
        .find(|w| w.default || w.id == list.default)
        .map_or_else(|| list.default.clone(), |w| w.name.clone())
}

/// True when a failed read named a workspace the hub no longer has (it was
/// removed here, from the CLI or from another desk).
pub fn read_lost_workspace(error_body: &str) -> bool {
    serde_json::from_str::<serde_json::Value>(error_body)
        .ok()
        .and_then(|v| v.get("error").and_then(|e| e.as_str()).map(str::to_string))
        .is_some_and(|code| code == "unknown_workspace")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::derive::workspace::test_words::{en, es};

    fn tile(id: &str, current: bool) -> WorkspaceTile {
        WorkspaceTile {
            id: id.into(),
            name: id.to_uppercase(),
            tile: id[..1].to_uppercase(),
            current,
            ..Default::default()
        }
    }

    fn entry(id: &str, default: bool) -> WorkspaceEntry {
        WorkspaceEntry {
            id: id.into(),
            name: format!("{id} name"),
            default,
            ..Default::default()
        }
    }

    #[test]
    fn a_name_is_kept_the_way_the_hub_keeps_it() {
        assert_eq!(clean_name("  Client   work \n"), "Client work");
        assert!(name_ok(" Client work "));
        assert!(!name_ok("   "));
        assert!(name_ok(&"x".repeat(NAME_MAX)));
        assert!(!name_ok(&"x".repeat(NAME_MAX + 1)));
    }

    #[test]
    fn an_empty_name_is_said_only_after_a_try() {
        assert_eq!(name_problem("", false, &en()), None);
        assert!(name_problem(" ", true, &en()).is_some());
        assert_eq!(name_problem("Ops", true, &en()), None);
    }

    #[test]
    fn steps_go_name_owners_ready_and_back_only_once() {
        assert_eq!(Step::Name.next("  "), None, "no name, no next step");
        assert_eq!(Step::Name.next("Client work"), Some(Step::Owners));
        assert_eq!(Step::Owners.next(""), Some(Step::Ready));
        assert_eq!(Step::Ready.next("x"), None);
        assert_eq!(Step::Owners.back(), Some(Step::Name));
        assert_eq!(Step::Name.back(), None);
        assert_eq!(Step::Ready.back(), None, "created: nothing to undo here");
        for (step, w) in Step::ALL.iter().zip([en(), es()].iter().cycle()) {
            assert!(!w.text(step.title_key(), &[]).starts_with("shell."));
        }
    }

    fn known(owner: &str, places: &[(&str, u64)], claimed_by: Option<&str>) -> KnownOwner {
        use crate::model::workspaces::OwnerPlace;
        KnownOwner {
            owner: owner.into(),
            repos: places.iter().map(|(_, n)| n).sum(),
            from: places
                .iter()
                .map(|(w, n)| OwnerPlace {
                    workspace: w.to_string(),
                    repos: *n,
                })
                .collect(),
            claimed_by: claimed_by.map(str::to_string),
        }
    }

    fn hub() -> WorkspacesList {
        let mut me = entry("me", true);
        me.name = "Personal".into();
        let mut acme = entry("acme", false);
        acme.name = "Acme".into();
        WorkspacesList {
            default: "me".into(),
            workspaces: vec![me, acme],
            owners: vec![
                known("Arakiss", &[("me", 4)], Some("me")),
                known("acme", &[("acme", 2)], Some("acme")),
                known("globex", &[("me", 1)], None),
            ],
            ..Default::default()
        }
    }

    #[test]
    fn picking_owners_says_how_many_repositories_move_and_from_where() {
        let list = hub();
        assert_eq!(move_of(&list, &[]), None);
        let one = move_of(&list, &["GLOBEX".into()]).unwrap();
        assert_eq!(
            one,
            Move {
                repos: 1,
                from: vec!["Personal".into()]
            }
        );
        assert_eq!(
            move_summary(Some(&one), false, &en()).as_ref(),
            "1 repository will move here from Personal."
        );
        let both = move_of(&list, &["acme".into(), "Arakiss".into()]).unwrap();
        assert_eq!(both.repos, 6);
        assert_eq!(both.from, vec!["Personal".to_string(), "Acme".to_string()]);
        assert_eq!(
            move_summary(Some(&both), true, &en()).as_ref(),
            "6 repositories moved here from Personal and Acme."
        );
        assert!(!move_summary(None, false, &es()).starts_with("shell."));
        assert_eq!(
            owner_meta(&list, &list.owners[0], &en()).as_ref(),
            "4 repositories · Personal"
        );
        assert_eq!(
            join_names(&["A".into(), "B".into(), "C".into()], &es()),
            "A, B y C"
        );
    }

    #[test]
    fn a_pick_toggles_whatever_the_case() {
        let mut picked = Vec::new();
        toggle_owner(&mut picked, "Acme");
        assert_eq!(picked, vec!["Acme"]);
        toggle_owner(&mut picked, "acme");
        assert!(picked.is_empty());
    }

    #[test]
    fn the_preview_uses_the_id_serve_will_give() {
        let list = hub();
        assert_eq!(predicted_id(&list, "  Client   work "), "client-work");
        assert_eq!(predicted_id(&list, "Acme"), "acme-2");
        assert_eq!(
            predicted_id(&WorkspacesList::default(), "harbard"),
            "harbard-2"
        );
    }

    #[test]
    fn refusals_are_worded_and_unknown_ones_keep_serves_text() {
        for w in [en(), es()] {
            for code in [
                "invalid_name",
                "removing_default",
                "last_workspace",
                "unknown_action",
            ] {
                let text = refusal(Some(code), "serve said", &w);
                assert_ne!(text.as_ref(), "serve said");
                assert!(!text.starts_with("shell."), "{code} has no wording");
            }
        }
        assert_eq!(
            refusal(Some("store_failed"), "disk full", &en()).as_ref(),
            "disk full"
        );
        assert_eq!(refusal(None, "unreachable", &en()).as_ref(), "unreachable");
    }

    #[test]
    fn the_menu_lists_every_workspace_in_order_with_the_current_checked() {
        assert!(workspace_rows(&[], &en()).is_empty());
        let one = workspace_rows(&[tile("a", true)], &en());
        assert_eq!(one.len(), 1, "a single workspace is listed too");
        assert!(one[0].current);
        let mut many: Vec<WorkspaceTile> =
            (0..12).map(|i| tile(&format!("w{i:02}"), i == 5)).collect();
        many[7].working = 3;
        let rows = workspace_rows(&many, &en());
        let ids: Vec<&str> = rows.iter().map(|r| r.id.as_str()).collect();
        assert_eq!(ids.len(), 12);
        assert_eq!(
            ids.first(),
            Some(&"w00"),
            "the hub's order, not where you are first"
        );
        let current: Vec<&str> = rows
            .iter()
            .filter(|r| r.current)
            .map(|r| r.id.as_str())
            .collect();
        assert_eq!(current, vec!["w05"]);
        assert_eq!(rows[7].label.as_ref(), "W07 · 3 working");
        assert_eq!(rows[6].label.as_ref(), "W06");
    }

    #[test]
    fn only_a_workspace_that_is_not_the_default_can_go() {
        let list = WorkspacesList {
            default: "me".into(),
            workspaces: vec![entry("me", true), entry("acme", false)],
            ..Default::default()
        };
        assert!(!removable(&list, &list.workspaces[0]));
        assert!(removable(&list, &list.workspaces[1]));
        assert_eq!(default_name(&list), "me name");
        let alone = WorkspacesList {
            default: "me".into(),
            workspaces: vec![entry("acme", false)],
            ..Default::default()
        };
        assert!(!removable(&alone, &alone.workspaces[0]));
    }

    #[test]
    fn a_read_for_a_removed_workspace_is_recognised() {
        assert!(read_lost_workspace(
            r#"{"error":"unknown_workspace","message":"no workspace has the id \"x\""}"#
        ));
        assert!(!read_lost_workspace(r#"{"error":"workspace_forbidden"}"#));
        assert!(!read_lost_workspace("serve is down"));
    }
}
