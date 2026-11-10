# @harbard/huginn

Views: what an agent shows the operator when a sentence would not do. Huginn is
the raven that brings Odin the news.

A view is a JSON file with the extension `.huginn`, attached to a note. The
desktop app renders it under the message (`apps/desktop/src/huginn`). The
catalog is written here once; the CLI embeds what `bun run generate` makes of
it, and `huginnCatalog` describes it to
[json-render](https://github.com/vercel-labs/json-render).

## What is here

| File | Holds |
| --- | --- |
| `src/catalog.ts` | Every component: props, what it may contain, its description |
| `src/parse.ts` | `parseView`, the one door into a view |
| `src/expression.ts` | Expressions, conditions and initial state |
| `src/answer.ts` | The text of an answer, and reading it back from the bus |
| `src/wire.ts` | What the Rust CLI embeds, generated from the catalog |
| `fixtures/` | Valid and invalid views; both validators run them |

## The rules that matter

- An answer holds ids, option numbers and what the operator typed, never a word
  the agent wrote. Nothing an agent puts on screen can reach another agent inside
  a note signed by the operator. The console reads it back in words against the view.
- State stays in the view. Expressions and conditions are a closed subset of
  json-render's (`src/expression.ts`): no `$computed`, `$bindState`, `on` or
  `watch`. A value read from state passes the prop's schema at render.
- `catalog.validate` from json-render only checks component names. `parseView`
  checks props, children, links, hidden characters and the shape of the tree.
- The console validates every view again before rendering. The check in
  `harbard` exists so the agent hears about a mistake while it can fix it.

## Changing the catalog

1. Edit `src/catalog.ts`, and draw the component in the app
   (`apps/desktop/src/huginn`).
2. `bun run generate` rewrites `crates/harbard-cli/src/huginn/`. A test fails if
   you forget, or if a schema uses a keyword the Rust validator does not know.
3. Add the component to `fixtures/valid.json`; a test fails if one is missing.

