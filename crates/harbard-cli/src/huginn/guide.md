# Huginn: show the operator a view instead of a sentence

A view is a JSON file with the extension .huginn, attached to a note. The note's
text stays the message; the view renders under it in the app.

  harbard huginn check view.huginn
  harbard note "..." --attach view.huginn --attach chart.png

{ "root": "card", "elements": {
    "card":  { "type": "Card", "props": { "title": "Deploy" }, "children": ["state"] },
    "state": { "type": "Metric", "props": { "label": "Errors", "value": "0" } } } }

Rules: root plus a flat map of elements; each child has one parent. An unknown prop
is refused. There are no actions and no watchers.
Answers: Choice, Form and each *Field need an id. The note you get holds no text of
yours, only ids and numbers: "plan → option 2" is the second option of Choice plan;
"reason → text" is what the operator typed; "(proposal unchanged)" means your
defaultValue was left alone. Options count from 1.
Files: Image, BeforeAfter and Annotated name files attached in the same command.
Limits: 400 elements, 12 levels, 262144 bytes.

State is optional and stays inside the view:
  { "root": "s", "state": { "env": "dev", "rows": [{ "name": "api", "ok": true }] },
    "elements": { "s": { "type": "Stack", "props": {}, "children": ["pick", "list"] },
      "pick": { "type": "Segmented", "props": { "state": "/env", "options": [
        { "label": "Dev", "value": "dev" }, { "label": "Prod", "value": "prod" }] } },
      "list": { "type": "Stack", "props": {}, "children": ["row"],
        "repeat": { "statePath": "/rows", "key": "name" },
        "visible": { "$item": "ok", "eq": true } },
      "row": { "type": "Text", "props": { "text": { "$template": "${name} en ${/env}" } } } } }
A prop may be a whole expression: $state, $item, $index, $template, or
$cond/$then/$else. Not inside lists, and not in control, answer or file props.
visible hides an element: { "$state": "/x", "eq": 1 }, gt, gte, lt, lte, neq, not,
$and, $or. On a repeat it filters items. A *Field with state writes its value there,
so one field can show another. One repeat per branch, 30 elements inside,
200 items. Answer and control components cannot repeat.

Use a view when the shape carries meaning: a table, a figure, a decision. A short
answer stays text.

## layout: arrange other components
  Stack { direction?: "vertical"|"horizontal", gap?: "sm"|"md"|"lg", align?: "start"|"center"|"end"|"stretch", wrap?: boolean } +children
    Children one after another.
  Grid { columns: int, gap?: "sm"|"md"|"lg" } +children
    Children in equal columns that collapse on a narrow pane.
  Section { title: string, description?: string } +children
    A titled part of a longer view.
  Card { title?: string, subtitle?: string, tone?: "neutral"|"accent"|"success"|"warning"|"danger"|"info" } +children
    A bordered box; tone colours its edge.
  Tabs { labels: string[] } +children
    One tab per child, in order; labels and children must match in number.
  Disclosure { label: string, open?: boolean } +children
    Content folded behind a label: detail most readers skip.
  Separator {  }
    A horizontal rule.

## text: words, code and links
  Heading { text: string, level?: int }
    A heading.
  Text { text: string, tone?: "default"|"muted"|"success"|"warning"|"danger", size?: "sm"|"md"|"lg", strong?: boolean }
    A paragraph of plain text.
  Markdown { source: string }
    Headings, lists, tables, code and links in Markdown. Raw HTML stays text.
  Callout { tone: "neutral"|"accent"|"success"|"warning"|"danger"|"info", title?: string, text: string }
    A note that must not be missed.
  Quote { text: string, cite?: string }
    Someone's words, with who said them.
  Badge { text: string, tone?: "neutral"|"accent"|"success"|"warning"|"danger"|"info" }
    A short state word.
  KeyValue { items: { label: string, value: string }[] }
    Label and value pairs.
  List { items: string[], ordered?: boolean }
    A bulleted or numbered list of short lines.
  Link { href: http_url, label: string }
    An http or https link, opened outside the app.
  Code { code: string, language?: string, filename?: string }
    A block of code in monospace.

## data: figures, tables and states
  Metric { label: string, value: string, delta?: string, deltaTone?: "neutral"|"success"|"danger", hint?: string }
    One headline figure. Prefer it to a chart with one number.
  Table { caption?: string, columns: { label: string, align?: "start"|"end" }[], rows: string|number|null[][], sortable?: boolean }
    Rows and columns; each row has one cell per column. sortable adds sorting by column.
  Progress { label: string, value: number, max?: number, tone?: "neutral"|"accent"|"success"|"warning"|"danger"|"info", hint?: string }
    How far along something is.
  Timeline { items: { time?: string, title: string, detail?: string, tone?: "neutral"|"accent"|"success"|"warning"|"danger"|"info" }[] }
    What happened, in order.
  Steps { items: { label: string, state: "done"|"current"|"todo"|"failed", detail?: string }[] }
    A sequence with where it stands now.
  Checklist { items: { label: string, state: "pass"|"fail"|"warn"|"skip", detail?: string }[] }
    Checks with their verdicts.
  Compare { title?: string, options: string[], criteria: { label: string, cells: { text: string, tone?: "neutral"|"accent"|"success"|"warning"|"danger"|"info" }[] }[], recommended?: int }
    Two to four options side by side against criteria, for a decision; each criterion has one cell per option. recommended counts options from 1.
  Board { columns: { title: string, tone?: "neutral"|"accent"|"success"|"warning"|"danger"|"info", cards: { title: string, detail?: string, tag?: string, tone?: "neutral"|"accent"|"success"|"warning"|"danger"|"info" }[] }[] }
    Columns of cards: where each piece of work stands; up to 5 columns of 12 cards. Prefer it to a Table when the column is the state.
  Bullet { label: string, value: number, target: number, max?: number, unit?: string, lowerIsBetter?: boolean, hint?: string }
    One measured value against its target, with the verdict. Prefer it to Progress when there is a target to meet; max defaults to 1.25 times the larger of value and target.
  Tree { nodes: { label: string, depth: int, detail?: string, tone?: "neutral"|"accent"|"success"|"warning"|"danger"|"info", state?: "done"|"current"|"todo"|"failed" }[] }
    An outline by depth (0 to 6, at most 80 nodes): a breakdown or a chain of dependencies. A node's state shows the mark Steps uses.

## chart: plots (at most 8 series)
  BarChart { title: string, labels: string[], series: { name: string, values: number|null[] }[], unit?: string, stacked?: boolean }
    Compare amounts across categories.
  LineChart { title: string, labels: string[], series: { name: string, values: number|null[] }[], unit?: string, area?: boolean }
    Change over an ordered axis; null leaves a gap.
  Sparkline { label: string, values: number[], unit?: string }
    A small inline trend beside a label.
  Donut { title: string, segments: { label: string, value: number }[], unit?: string }
    Parts of a whole; at most 6 parts.
  Heatmap { title: string, rows: string[], columns: string[], values: number|null[][], unit?: string }
    A matrix of amounts, darker for more.

## work: diffs, files, terminals, images attached with the view
  Diff { source: string, filename?: string }
    A unified diff with its colours.
  FileTree { entries: { path: string, status?: "added"|"modified"|"deleted"|"renamed", note?: string }[] }
    Paths and what happened to each.
  Terminal { command?: string, output: string, exitCode?: int }
    A command and what it printed.
  Image { file: file_name, alt: string, caption?: string }
    An image attached in the same command, by file name.
  BeforeAfter { before: file_name, after: file_name, mode?: "side"|"slider" }
    Two attached images to compare, side by side or under a slider.
  Annotated { file: file_name, alt: string, caption?: string, pins: { x: number, y: number, label: string, detail?: string, tone?: "neutral"|"accent"|"success"|"warning"|"danger"|"info" }[] }
    An attached image with up to 20 numbered pins and their legend: findings marked where they are. x and y are fractions of its width and height.

## control: change the view's own state; nothing leaves the view
  Segmented { state: state_path, label?: string, options: { label: string, value: string }[] }
    Pick one value for state/path: tabs of a filter.
  Switch { state: state_path, label: string }
    Turn a boolean at state/path on or off.

## answer: ask the operator; the answer comes back as a note
  Choice { id: answer_id, question: string, detail?: string, multiple?: boolean, options: { label: string, detail?: string, tone?: "neutral"|"primary"|"danger" }[] }
    One question with options. A click on an option is the answer; multiple adds a send button.
  Form { id: answer_id, title: string, detail?: string, submitLabel?: string } +children
    Fields answered together. Its children hold the *Field components.
  TextField { id: answer_id, label: string, hint?: string, required?: boolean, state?: state_path, placeholder?: string, multiline?: boolean, defaultValue?: string }
    Free text; multiline for a paragraph.
  NumberField { id: answer_id, label: string, hint?: string, required?: boolean, state?: state_path, min?: number, max?: number, step?: number, unit?: string, defaultValue?: number }
    A number within bounds.
  SelectField { id: answer_id, label: string, hint?: string, required?: boolean, state?: state_path, options: string[], defaultValue?: string }
    One of many options, in a menu.
  RadioField { id: answer_id, label: string, hint?: string, required?: boolean, state?: state_path, options: string[], defaultValue?: string }
    One of a few options, all visible.
  CheckboxField { id: answer_id, label: string, hint?: string, state?: state_path, defaultChecked?: boolean }
    Yes or no.
  MultiSelectField { id: answer_id, label: string, hint?: string, required?: boolean, state?: state_path, options: string[], defaultValue?: string[] }
    Any number of options.
  DateField { id: answer_id, label: string, hint?: string, required?: boolean, state?: state_path, defaultValue?: iso_date }
    A day; values are YYYY-MM-DD.
  SliderField { id: answer_id, label: string, hint?: string, state?: state_path, min: number, max: number, step?: number, unit?: string, defaultValue?: number }
    A value on a range.
