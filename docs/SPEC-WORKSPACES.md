# Workspaces

A workspace is an organisation: a name, an icon (the rail tile), and a rule that
claims repositories. Each repository belongs to exactly one workspace. Channels,
agents (by the repository they work in), notes, lanes, Deliveries and Activity
are scoped to a workspace.

## Registry

`{home}/workspaces.json` (distinct from `workspace.json`, the desk) holds the
workspaces (`id`, `name`, `icon`, `owners`), the `default`, and the repositories
moved by hand (`repos`: repository identity → id). Without the file the hub has
one implicit workspace and nothing is scoped.

Serve seeds the file once at start, and `harbard workspace` does too: one
workspace per origin owner in the history. The owner `gh` is signed in as
becomes "Personal" (id `personal`) and the default; others keep their owner's
name until renamed. An unreadable file is never overwritten, and delivery then
separates by origin owner rather than opening every boundary.

## Placement

The rule, in order:

1. A move by hand.
2. The workspace claiming the owner of the repository's `origin`, read from the
   git files, with worktrees and subdirectories resolved to their checkout
   (`<org-a>/*` to one workspace, `<org-b>/*` to another).
3. The default workspace.

A satellite granted exactly one workspace has the checkouts this hub cannot read
placed there. Placement is computed at read time: the log is never rewritten,
and moving a repository moves its history.

## Bus

`Reader` carries a required `workspace` gate; an event is delivered only inside
its writer's workspace. A note to a session or repository of another workspace
is refused at write with the reason. `brief` does not count or quote another
workspace's live sessions. The operator's notes are never gated, the console
scopes its own targeting, except a note written in a Room: it is placed in the
workspace it was written in and read and delivered only there.

## Serve

`GET /api/v1/workspace?ws=<id>` reads one workspace and carries the rail's
tiles. Every entry of `channels` has `kind` (`project` or `room`), `purpose`,
`archived` and `built_in`; a Room has no repository, checkouts or warnings. The
view also carries `lobby`: one card per main session and named agent, newest
first, with the latest thing each said in `#lobby`. Writes resolve targets in the workspace named in the body.
`GET|POST /api/v1/workspaces[/add|remove|move|reset|rename|icon]` manage the registry from
the desk. The read also lists the origin owners the history knows, each with how many
repositories a claim would bring and where they are now; `add` takes the name with its
owners and icon in one change. History is per workspace.

`/feed`, `/search` and `/team` read the whole hub and only Operator,
which sees every workspace, reaches them. `/events` and `/stream` are the
transport a satellite and the console's invalidation use.

## Devices

`harbard hub invite --workspace <id>` grants a workspace at pairing, and
`harbard hub grant <device> <workspace>...` changes it later. No grant means the
default workspace only. Operator sees every workspace: it can edit the grants
and the registry, so limiting it would protect nothing. Workshop reads and
Display summaries are cut to the grants.

## Console

- **Rail.** One tile per workspace, the current one first and largest and the
  others smaller below it. A tile selects its workspace and shows a count for
  "Needs you", a dot for unread, and a green dot when its agents are running.
  The choice is a cookie.
- **Sidebar head.** The workspace name opens its menu (Preferences, Themes,
  Shortcuts, Retro, About), starts with "Switch to <workspace>", and leads to
  "<workspace> settings" (name, tile, repositories and "Move to…").
- **Emblem.** Every workspace has one generated from its id (four flat shapes on
  a 2×2 grid), so a rename keeps it. Two workspaces never share a palette while
  there are palettes to go round. A text set in Settings is drawn over it.
- **Empty places.** A channel or sidebar with nobody working in it names the
  busiest other workspace and goes there on a tap.
- **Strip view.** Reads every workspace (`?ws=all`) and has no switcher. Each
  agent and card says its workspace, and a tap moves the main window there. A
  device that cannot see every workspace reads as if it had not asked.

## Not in scope

- Workspaces on different hubs: one hub holds many workspaces.
- Per-workspace themes or operator profiles.
