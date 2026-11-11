# Huginn

Views: what an agent shows the operator when a sentence would not do. Huginn is
the raven that brings Odin the news.

A view is a JSON file with the extension `.huginn`, attached to a note. The
desktop app renders it under the message (`apps/desktop/src/huginn`). The
catalog of components lives here, once, and both sides read it.

## What is here

| File | Holds |
| --- | --- |
| `catalog.json` | Every component: family, description, props as JSON Schema, what it may contain. Edited by hand. |
| `../huginn.rs` | `validate`, the one door into a view, and `resolved_props_problem` for props read from state |
| `guide.rs` | Writes the guide agents read in `harbard huginn catalog` from the catalog |
| `guide.md` | That guide, checked in so a catalog change shows what agents will read |
| `../../tests/fixtures/huginn/` | Valid and invalid views; the CLI and the app both run them |

## The rules that matter

- An answer holds ids, option numbers and what the operator typed, never a word
  the agent wrote. Nothing an agent puts on screen can reach another agent inside
  a note signed by the operator. The app reads it back in words against the view
  (`apps/desktop/src/huginn/answer.rs`).
- State stays in the view. Expressions and conditions are a closed subset:
  `$state`, `$item`, `$index`, `$template` and `$cond`; no `$computed`,
  `$bindState`, `on` or `watch`. A value read from state passes the prop's
  schema at render (`resolved_props_problem`).
- `validate` checks component names, props, children, links, hidden characters
  and the shape of the tree, within the catalog's `limits`.
- The app validates every view again before rendering. The check in `harbard`
  exists so the agent hears about a mistake while it can fix it.

## The catalog

`catalog.json` holds `limits` (the bounds of a whole view), `families` (in the
order the guide lists them) and `components` (in the order the guide lists
them within a family). Each component has:

- `family` and a one-line `description`;
- `children`: `"none"`, `"some"`, or `{ "sameLengthAs": "<list prop>" }`;
- `insideForm` (a field, only valid inside a Form) or `answers` (Choice, Form);
- `files`: props that name a file attached with the view;
- `lengths`: list props whose items must match another list in length;
- `props`: a JSON Schema object with `"additionalProperties": false`. Only the
  keywords the validator applies are allowed, and a value JSON Schema cannot
  express carries `"x-huginn-format"`: `http_url`, `file_name`, `iso_date`,
  `answer_id` or `state_path`.

Which props must be plain values is not written down: answer, field and control
components are fixed whole, and so are file props and the list Tabs counts.

## Changing the catalog

1. Edit `catalog.json`, and draw the component in the app
   (`apps/desktop/src/huginn`).
2. `just huginn-guide` writes `guide.md` again. A test fails if you forget, if a
   schema uses a keyword or format the validator does not apply, or if a
   component contradicts itself (an unknown family, a file prop that is not a
   `file_name`, an answer without an `id`).
3. Add the component to `tests/fixtures/huginn/valid.json`; a test fails if one
   is missing. Add a view it must refuse to `invalid.json`, with the issue code
   it must be refused for.
