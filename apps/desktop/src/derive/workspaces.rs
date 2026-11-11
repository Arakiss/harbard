//! The hub's workspaces as the operator edits them from the app: creating
//! one from a name, removing one, and who the workspace menu offers to
//! switch to. serve picks the id and refuses what the registry forbids
//! (`serve/workspaces_route.rs`); this holds what the dialogs say before
//! and after sending.

use super::workspace::Words;
use crate::model::WorkspaceTile;
use crate::model::workspaces::{WorkspaceEntry, WorkspacesList};
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

/// The owners typed in one field: commas, spaces or new lines between
/// them, a leading `@` dropped, each owner once whatever its case.
pub fn parse_owners(raw: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for part in raw.split(|c: char| c == ',' || c == ';' || c.is_whitespace()) {
        let owner = part.trim().trim_start_matches('@');
        if !owner.is_empty() && !out.iter().any(|o| o.eq_ignore_ascii_case(owner)) {
            out.push(owner.to_string());
        }
    }
    out
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
    fn owners_split_on_commas_spaces_and_lines_once_each() {
        assert_eq!(
            parse_owners("acme, Globex\n@initech;  ACME\n\n"),
            vec!["acme", "Globex", "initech"]
        );
        assert!(parse_owners("  , \n").is_empty());
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
