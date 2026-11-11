# Design rules for the native app

These rules govern every pixel `harbard-app` draws. They exist because the console's shape is
settled (the rail, the panel, the screens and where things sit in them) and the remaining work is
craft: type, spacing, colour discipline, states, motion, density and the keyboard. Nothing here
moves a region, renames a screen or adds a layout. A change that needs any of those is out of scope
for this document.

The reference for craft is Linear. The reference is its discipline, not its look: harbard keeps its
own fonts, its own themes and its own warmth. Section 1 records what Linear does as numbers, section
2 turns that into harbard's tokens, section 3 says where harbard differs on purpose.

How to read a rule: every rule has an id (`T2`, `C4`…). The audit in section 6 and any review
comment cite the id. A rule with a number in it is checked with a ruler on a capture, not by eye.

## 1. What Linear does, measured

Each row says where the value comes from:

- **documented**: Linear wrote it. Sources:
  [L1](https://linear.app/now/how-we-redesigned-the-linear-ui),
  [L2](https://linear.app/now/behind-the-latest-design-refresh),
  [L3](https://linear.app/changelog/2024-03-20-new-linear-ui),
  [L4](https://linear.app/method/introduction),
  [L5](https://linear.app/developers/aig),
  [L6](https://linear.app/docs/inbox),
  [L7](https://linear.app/changelog/2019-09-23-improved-peek),
  [L8](https://linear.app/now/design-for-the-ai-age),
  [L9](https://linear.app/docs/account-preferences).
- **measured**: read from the stylesheet and markup of <https://linear.app> on 2026-11-11. That page
  embeds a working replica of the app (sidebar, issue list, issue page, inbox, command menu), so its
  CSS carries real component sizes. The signed-in app's own stylesheet was not read.
- **observed**: seen while using the product; an estimate, to the nearest 2 px.

### 1.1 Type

| What | Value | Source |
|---|---|---|
| Interface face | Inter Variable, features `cv01`, `ss03`, optical size auto | measured |
| Headings | Inter Display "to add more expression to our headings", Inter for the rest | documented L1 |
| Code | Berkeley Mono | measured |
| Sizes in the app replica | 13 px is the most used (24 rules), then 12 px (20); 14, 15 and 10–11 px are rare | measured |
| Named steps | tiny 10, micro 12, mini 13, small 14, regular 15, large 17 | measured |
| Titles inside the app | 17, 20, 24 px at weight 590 | measured |
| Weights | 400, 510 ("medium"), 590 ("semibold"); 680 exists and is almost unused. Medium is the workhorse: 20 rules against 4 for semibold | measured |
| Tracking | −0.010 em at 13 px, −0.013 em at 14, −0.011 em at 15, −0.012 em on titles to 24 px, −0.022 em from 32 px up, 0 at 12 px | measured |
| Line height | 1.5 for 13 px, 21/14 for 14 px, 1.6 for 15 px; titles 1.33–1.4 | measured |
| Hierarchy carrier | Four text colours (primary to quaternary), not four weights | measured |

### 1.2 Sizes and spacing

| What | Value | Source |
|---|---|---|
| Sidebar row | 28 px high, radius 8, 7 px inline padding, 8 px icon gap, 2 px between rows, 13 px medium | measured |
| List row (issues) | 40 px, hairline on top, no card | measured |
| Group header in a list | 36 px (24 px compact), radius 8, a 2 % white wash, no border | measured |
| Property and activity rows | 24 px, 6–8 px gap | measured |
| View header | 44 px, hairline below | measured |
| Menu item | 32 px, 13 px text, radius 6, 10–14 px inline padding | measured |
| Command-menu row | 46 px min, 12 px gap, selection drawn as a radius-8 wash inset 2 px | measured |
| Icon button | 28 px or 24 px square around a 16–20 px icon | measured |
| Label chip | 24 px high, hairline border, fully rounded, 12 px text, 6–8 px inline padding | measured |
| Count badge on an agent | 22 px high, radius 4, 10 px text | measured |
| Gaps in use | 8 px (35 rules), 6 (35), 4 (21), 12 (15), 16 (9), 2 (8), 24 (6). No 5, 7, 9, 10 or 14 worth counting | measured |
| Radii | 4, 6, 8, 12, 16, 24, 32 and full. 8 is the default (37 rules), then 6 (24) and 4 (16) | measured |
| Icons | 14 px (85 uses) and 16 px (81); 12 px for inline marks; 20 px is rare (9). Stroke 1.5 where stroked | measured |
| Inbox | A list beside the selected item; the 2024 redesign gave it "increased density and better contrast" | documented L3, L6 |

### 1.3 Colour

| What | Value | Source |
|---|---|---|
| Theme inputs | Three per theme: base, accent, contrast. Before the redesign it was 98 variables. Generated in LCH so equal lightness reads as equal across hues | documented L1 |
| Neutrals | "Limiting how much chrome (blue in our case)" enters the neutrals, for "a more neutral and timeless appearance"; later moved toward "a warmer gray that still feels crisp, but less saturated" | documented L1, L2 |
| Contrast | Text and neutral icons darker in light mode, lighter in dark mode | documented L1 |
| Surfaces | Four background levels a few lightness points apart (dark: `#08090a`, `#0f1011`, `#141516`, `#191a1b`). Elevation in dark mode is a lighter surface, not a shadow | measured |
| Borders | Hairlines at 5–8 % white (dark) or 5–8 % black (light), 1 px (0.5 px on dense displays) | measured |
| Accent | One hue, on the primary button, the focus ring, links and selection. Everything else is neutral | measured |
| State colours | Green, yellow, orange, red, blue appear only as 8–14 px marks or as text, never as a filled region | measured, observed |
| Sidebar | "A few notches dimmer, allowing the main content area […] to take precedence" | documented L2 |
| Team icon tiles | Coloured backgrounds behind icons were removed as "unnecessary visual treatments" | documented L2 |
| Separators | Had "quietly proliferated"; the refresh removed many and softened the rest | documented L2 |

### 1.4 Elevation

| What | Value | Source |
|---|---|---|
| Shadow steps (light) | tiny `0 1px 1px` 9 %, low `0 1px 4px −1px` 9 %, medium `0 3px 12px` 9 %, high `0 7px 24px` 6 % | measured |
| Shadow steps (dark) | low `0 2px 4px` 10 %, medium `0 4px 24px` 20 %, high `0 7px 32px` 35 % | measured |
| Where shadows go | Popovers, menus, dialogs, the command menu. Rows, cards in a list and panes have none | measured |
| Stacking | popover 600, command menu 650, dialog 700, toasts 800, tooltip 1100, context menu 1200 | measured |

### 1.5 Motion

| What | Value | Source |
|---|---|---|
| Quick transition | 100 ms | measured |
| Regular transition | 250 ms | measured |
| Hover and colour changes | 160 ms, `cubic-bezier(.25,.46,.45,.94)` (ease-out quad); 120 ms on small marks | measured |
| Row highlight | Appears in 0 s, fades out in 150 ms | measured |
| Press | `scale(.97)` on the primary button, 160 ms | measured |
| Size changes | 220 ms for a height or width change | measured |
| What never animates | Layout of lists, text content, selection moving between rows, 12 rules set `transition: none` explicitly | measured |
| Reduced motion | Honoured (`prefers-reduced-motion` turns transitions off) | measured |

### 1.6 States and the keyboard

| What | Value | Source |
|---|---|---|
| Hover on a row | A 2 % wash (sidebar), no border, no movement | measured |
| Active sidebar row | A 4 % wash; the label does not change weight | measured |
| Focus | A ring: 1 px inset on list rows, 2 px with 2 px offset elsewhere, drawn only for keyboard focus | measured |
| Selected in a menu | Text goes from secondary to primary and a level-3 surface appears behind it | measured |
| Unread in the inbox | A dot at the row's leading edge and primary text; read rows drop to secondary text | observed |
| Checkboxes in a list | Shown on hover, focus or selection only | observed |
| Shortcut key | A chip 20 px high, min 20 px wide, 11 px medium, radius 3–5, hairline border, tertiary text | measured |
| Where keys are shown | At the right edge of command-menu rows and menu items, and after the label in tooltips. Not on buttons, not in body copy | measured, observed |
| Inbox keys | `G` then `I` opens it; `J`/`K` or arrows move; `U` read/unread; `H` snooze; `Backspace` delete; `Esc` clears | documented L6 |
| Peek | `Space` previews the focused row; holding it previews while held | documented L7 |
| Comment submit | The person chooses whether `Enter` or `Cmd`+`Enter` sends | documented L9 |

### 1.7 Principles behind the numbers

| Principle | Source |
|---|---|
| "Reduce visual noise, maintain visual alignment, and increase the hierarchy and density of navigation elements." Labels, icons and buttons were aligned vertically and horizontally across the sidebar and tabs; the result is "something that you'll feel after a few minutes" | documented L1 |
| Parts "that support orientation and navigation should recede" | documented L2 |
| "The refresh reduces icon usage, scales their sizes down" | documented L2 |
| Header actions must sit in predictable places from screen to screen | documented L2 |
| Density is kept "without letting the interface feel overwhelming" | documented L2 |
| "Aim for clarity": plain, consistent names. "Simple first, then powerful" | documented L4 |
| An agent "should always disclose that it's an agent", "provide instant feedback" and "be clear and transparent about its internal state" | documented L5 |
| Structure beats a blank chat box: a purpose-built surface with agents placed on it, their work reviewed there | documented L8 |

## 2. harbard's tokens

Colours stay exactly as they are: the `--hb-*` tokens served by the theme route, read through
`theme.c("name")`. The eleven themes and the seasons keep working because no rule below names a hex
value. What changes is everything that is not colour.

Constants live in `apps/desktop/src/ui/mod.rs` and `apps/desktop/src/ui/kit_theme.rs`. A screen
never writes a bare number for a size, a radius, a gap or a duration; it uses the constant. Today
the app's sources hold 18 distinct text sizes, 19 radii, 30 paddings, 26 heights and 19 animation
durations. After this document they hold the values in the tables below and no others.

### 2.1 Type

harbard's faces stay: Lato for the workspace, IBM Plex Mono for code, hashes and paths, Schibsted
Grotesk for headings outside the workspace (Setup and the Strip).

Lato draws smaller than Inter at the same size, so harbard's steps sit one pixel above Linear's:
harbard's 14 does the job of Linear's 13.

| Constant | px | Line height | Used for | Change |
|---|---|---|---|---|
| `size::MICRO` | 11 | 14 | Count badges, key chips | none |
| `size::META` | 12 | 16 | Timestamps, section labels, captions, chips | none |
| `size::SMALL` | 13 | 18 | Second lines, status lines, disclosures, hints | none |
| `size::UI` | 14 | 20 | Controls, sidebar rows, tabs, menu items, list-row titles | none |
| `size::BODY` | 15 | 22 | Message text and anything read as prose | none |
| `size::NAV` | 14 | 20 | Sidebar rows | **15 → 14** |
| `size::TITLE` | 18 | 24 | The one title of a pane or dialog | none |
| `size::DISPLAY` | 24 | 30 | Schibsted Grotesk headings outside the workspace | **28 → 24** (defined today, used nowhere) |

Add `pub mod leading` beside `size` with the line heights above, and make `ui::text()` set the line
height from the size, so no call site sets one.

- **T1. Eight sizes, no others.** 16, 17, 19, 20, 21, 22, 26, 32, 34 and 50 px leave the workspace
  screens. The Strip keeps its own scaled sizes because it is read from across a room.
- **T2. Three weights, because three are shipped.** Lato ships Regular (400), Bold (700) and Black
  (900). `SEMIBOLD` and `EXTRA_BOLD` have no file behind them and must not be requested.
  - 400: everything by default, including sidebar rows, body, second lines, hints.
  - 700: the title of a list row, an unread row, the selected tab, button labels, section labels.
  - 900: the pane title (`TITLE`), author and agent names, and nothing else. At most two kinds of
    900 text per pane.
  Today 82 call sites ask for Black and 129 for Bold against 36 for Regular; the target is the
  reverse order. Add `weight::{REGULAR, STRONG, HEAVY}` and use those names.
- **T3. Hierarchy by ink, then size, then weight.** Four inks: `ink` (what you read), `ink-2`
  (supporting prose), `ink-3` (metadata, labels), `ink-4` (placeholders, disabled; never below 12
  px). Reach for a heavier weight only after ink and size have been used.
- **T4. One title per pane, always `TITLE` 18/900,** vertically centred in the 56 px header. A list
  pane and the detail pane beside it share that baseline.
- **T5. No tracking, so no capitals.** The renderer has no letter-spacing. Uppercase labels set
  without tracking look cramped, so section labels are sentence case, `META` 12/700 in `ink-3`
  (`sidebar-ink-2` in the sidebar). Small sizes are never set in capitals.
- **T6. Mono is two pixels smaller than the text beside it** (13 beside `BODY`, 12 beside `UI` and
  `SMALL`) and is used only for things a person may copy: a hash, a path, a command, a branch.
- **T7. Text that can overflow declares how.** One line: `.truncate()`. Two lines: `.line_clamp(2).text_ellipsis()`; a clamp without the ellipsis stops in the middle of a word.
  Text is never cut by its container's edge.
- **T8. Numbers that change in place use tabular figures or a fixed-width box,** so a ticking
  duration does not shake the row.

### 2.2 Spacing

The unit is 4 px. Add `pub mod space` with these steps and no others:

| Constant | px | Typical use |
|---|---|---|
| `space::HAIR` | 2 | Between stacked sidebar rows; a badge against its icon |
| `space::XS` | 4 | Icon to its own label inside a chip; between a name and its tag |
| `space::SM` | 6 | Icon to label inside a control |
| `space::MD` | 8 | Between controls in a row; inside a row, between columns |
| `space::LG` | 12 | Avatar to text; padding inside inputs, banners, popovers |
| `space::XL` | 16 | Between blocks inside a pane; dialog padding |
| `space::GUTTER` | 20 | The left and right inset of every pane |
| `space::SECTION` | 24 | Between sections of a screen |
| `space::BREAK` | 32 | Above the first block of a settings page or an empty state |

- **S1. Only these steps.** 5, 7, 9, 10, 11, 14, 18, 22, 26 and 28 are rounded to the nearest step.
- **S2. One gutter.** Titles, list rows, message avatars, banners and the composer all start 20 px
  from their pane's edge. Put a ruler on the left edge of a capture: there is one line.
- **S3. Space separates before lines do.** Two blocks are separated by `SECTION` of space first. A
  hairline is added only between rows of the same list or under a header.
- **S4. Padding is symmetric** inside a control: the same top and bottom, the same left and right,
  unless an icon sits on one side.

### 2.3 Sizes

Add `pub mod row`, `pub mod control` and `pub mod icon_size`.

| Constant | px | What |
|---|---|---|
| `row::NAV` | 28 | Sidebar destination and channel rows (today 32) |
| `row::MENU` | 28 | Menu, popover and finder rows |
| `row::AGENT` | 44 | Two-line agent row in the sidebar (today 48) |
| `row::HEADER` | 56 | Pane and dialog header (unchanged) |
| `row::TABS` | 40 | Tab strip under a header |
| `control::SM` | 28 | Icon buttons, header buttons, filter fields in the sidebar |
| `control::MD` | 32 | Buttons, inputs, selects, search fields |
| `control::LG` | 36 | A primary action that stands alone (Setup, a view's ask). Buttons that share a row share one height: a dialog footer is `MD` throughout |
| `icon_size::INLINE` | 14 | Beside `META` and `SMALL` text |
| `icon_size::ROW` | 16 | Rows, controls, headers |
| `icon_size::RAIL` | 20 | The rail and the composer's send |
| avatar | 36 | Messages and list rows; 28 in sidebar agent rows; 20 in stacks |

- **Z1. No control is taller than 36 px** in the workspace. 44, 46 and 48 px buttons go.
- **Z2. A list row's height comes from its content plus `LG` (12) above and below;** rows of the
  same list have the same structure, so the same height.
- **Z3. Icons come in three sizes.** 11, 12, 18, 24 and 34–42 px icons leave the workspace. The
  icon set is Lucide, drawn with its 2 px stroke on a 24 px grid, which lands at 1.3 px at 16 and
  1.7 px at 20; two sizes never sit in the same row.
- **Z4. One avatar size per list.** 48, 72 and 104 px avatars leave the dense workspace lists; the
  character surfaces in §3.1 keep their large robots.

### 2.4 Radius

| Constant | px | What |
|---|---|---|
| `RADIUS_SM` (new) | 4 | Chips, tags, key chips, inline code, small swatches |
| `RADIUS` | 6 | Buttons, inputs, sidebar and menu rows, tabs |
| `RADIUS_MD` (new) | 8 | Avatars, banners, cards, popovers, the composer, and the main panel: it sits 6 px inside the window's own corner, so 8 keeps the two curves concentric (R2) |
| `RADIUS_LG` | 12 | Dialogs |
| `rounded_full` | — | Presence dots, count badges, the round rail buttons |

- **R1. Five radii.** 2, 3, 3.5, 5, 7, 9, 10, 11, 14, 16, 17, 22 and 35 go.
- **R2. A child's radius is never larger than its parent's,** and a pill (`rounded_full`) is never
  used for a button or a tag.

### 2.5 Colour, against the existing tokens

- **C1. Colour comes from `theme.c()` only.** No `hsla(…)`, `rgb(…)` or hex in a screen. Alpha is
  applied only to build a wash from a token that has no soft variant.
- **C2. Accent has four jobs:** the selected sidebar row (`accent`), the one primary button of a
  view (`accent-deep`), unread counts (`accent-deep`/`badge`), and links (`link`). A pane shows
  accent in at most three places at once. Accent is never a border, an outline, an icon tile or a
  heading.
- **C3. State colours mean state.** `green`, `amber`, `red` and their `-bg`/`-ink`/`-line` variants
  are used only when the thing is in that state: `red` is blocked or failed, `amber` is waiting on
  a person or a warning, `green` is working or verified. A step that is merely current, a count
  that is merely a count, or an unread mark is not a state and takes no state colour.
- **C4. Three surfaces.** `surface` (the pane you work in), `surface-2`/`list` (the pane beside
  it), `surface-raised` (what floats). A fourth background is a sign of a card that should not be
  there.
- **C5. Hairlines are `line`, 1 px.** `line-strong` is for the edge of a control. `line-hover` is
  for that edge under the pointer. No 2 px borders except the focus ring and the selected tab's
  underline.
- **C6. Border or shadow, never both,** and shadow only on what floats: `pop_shadow()` on menus,
  popovers, dialogs and the finder. Rows, cards and panes have no shadow.
- **C7. The sidebar recedes.** It uses `sidebar-ink` at weight 400; only the selected row and rows
  with something unread are 700. Section labels are `sidebar-ink-2`.
- **C8. Hover is `hover` (`sidebar-hover` in the sidebar, `rail-hover` on the rail);** selection in
  a list is `selected`; selection in a menu is `menu-hover` with `menu-hover-ink`. A row never
  shows two of these at once.
- **C9. Identity colours (`tone-1…8`, `robot-tile-1…4`) fill avatars only,** at 36 px or less. They
  never tint a card, a band or a header, except on the character surfaces in §3.1.

### 2.6 Motion

Add `pub mod motion`:

| Constant | ms | Easing | What |
|---|---|---|---|
| `motion::FAST` | 100 | `ease_out_quint` | Press feedback; a tooltip or a hover toolbar appearing |
| `motion::BASE` | 160 | `ease_out_quint` | A menu, popover or the finder appearing: opacity 0→1 and 4 px of travel |
| `motion::SLOW` | 240 | `ease_out_quint` | A dialog and its scrim; a pane opening or closing |
| `motion::ARRIVAL` | 150 | linear | The wash behind a row that just arrived, fading out |

- **M1. Three durations for interface motion** plus the arrival fade. The 19 durations in use today
  reduce to these; the looping marks in M4 keep their own periods.
- **M2. Hover and selection are immediate.** A hover style is a style swap; that is how the
  renderer works and it matches Linear's 0 s highlight. Nothing fades in on hover.
- **M3. These never animate:** a list reordering or growing, text changing, a count changing, the
  focus row moving with the keyboard, scroll position, a theme or mode change, a window resize.
  New content appears in its final place.
- **M4. A running animation keeps the window repainting,** so an idle screen has none. The only
  loops allowed are marks that report a live state: an agent's status icon, the robot's face, the
  mark's presence dot, and a season's decorations. Each stops when its window is not visible.
- **M5. No spinner for anything local.** The bus is read in-process; the screen shows the last
  known state at once and corrects it. A spinner (`loader-circle`) is allowed only for a wait on
  another process that exceeds one second: an update download, a pairing, a sign-in.
- **M6. Writes are optimistic.** A sent message, a mark-as-read, a resolve or a save shows its
  result immediately, with a quiet `META` line ("Sent") and an inline error with a retry if it
  fails.
- **M7. Reduced motion** (the General preference or the system setting) turns `BASE`, `SLOW` and
  `ARRIVAL` into no animation and holds every loop in M4 on its resting frame.

### 2.7 States

Every interactive element defines all of these. A capture of each is part of the checklist.

| State | Rule |
|---|---|
| Rest | No border on rows; `line-strong` on controls |
| Hover | Background token from C8, pointer cursor; nothing moves, nothing changes size |
| Pressed | The next background token down (`control`, `btn-hover`); no scale |
| Focus (keyboard) | **F1.** A 2 px ring in `focus` via `ring_shadow`, inset on rows, outside on controls. Every element with `.hover()` is also focusable: today there are 68 hover sites and 21 focus sites, and those numbers must match |
| Selected | `selected` in lists, `accent` in the sidebar; text stays the same size |
| Unread | **F2.** An 8 px `accent` dot at the leading edge and the row title at 700; the count, if any, in a 16 px badge at the trailing edge. Never a dot and a badge together |
| Disabled | Opacity 0.5, no hover, default cursor |
| Empty | **F3.** One line of `UI` 14/700 in `ink-2` and one sentence of `UI` 14/400 in `ink-3`, at most 320 px wide, starting `BREAK` (32) below the header, centred in the pane. No icon above 20 px, no illustration, at most one action |
| Loading | Per M5: the last known state, or the empty state. No skeleton for a read that takes under 200 ms |
| Error | Inline, where the action was, in `red-ink` on `red-bg`, with what failed and one action |

### 2.8 Keyboard

- **K1. Key chips** are 20 px high, at least 20 px wide, `MICRO` 11/700 in `ink-3`, radius
  `RADIUS_SM`, a 1 px `line` border, 4 px inline padding, 2 px apart.
- **K2. Keys appear in three places:** at the trailing edge of finder and menu rows, after the
  label in a tooltip, and in the shortcuts overlay. Never on a button face, never in a sentence.
- **K3. Every icon-only button has a tooltip** with its name and its key if it has one, drawn by
  the one piece in `kit_theme.rs` (`tip`, `tip_keys`; `tip_inside` for a control that lives in a
  dialog or the lightbox). It stays hidden while the popover its own button opened is showing.
- **K4. Lists move with the keyboard:** `J`/`K` and the arrows move a visible focus row, `Enter`
  opens, `Esc` goes back one level, `U` toggles read. These are the same keys in Activity, DMs,
  Later, Search, History and Deliveries.
- **K5. The composer's send key is shown once,** as a tooltip on the send button.

### 2.9 Shared helpers that change in `ui/mod.rs`

- `count_badge`: height 18 → 16, min width 18 → 16, inline padding 5 → 4, weight Black → 700.
- `dot`: sizes limited to 8 (lists, sidebar) and 10 (on a 36 px avatar).
- `empty`: becomes the two-line block in F3, top-aligned with `BREAK`, not centred vertically.
- `icon`: takes an `icon_size` constant, not a number.
- New `kbd(keys)`, `chip(label, tone)`, `section_label(text)` and `pane_header(title)` so that
  screens stop drawing their own.
- `kit_theme.rs`: `kit.font_size` `BODY` → `UI` (controls are 14); add `RADIUS_SM` 4 and
  `RADIUS_MD` 8; `mono_font_size` stays 13.

## 3. Where harbard departs from Linear, on purpose

Linear tracks work that changes a few times a day. harbard shows a fleet that changes every second.
These differences are deliberate and are not to be "fixed" toward Linear.

| harbard | Linear | Why, and the limit |
|---|---|---|
| A tinted rail and frame per theme, eleven themes | Neutral chrome | The frame is the brand and tells workspaces apart at a glance. Limit: the tint stays in the frame and rail; the panel inside is neutral |
| A solid accent fill on the selected sidebar row | A 4 % wash | It is the console's signature and survived two reviews. Limit: only that row; nothing else is filled with accent |
| Lato, 15 px body | Inter, 13 px | People read agents' prose here, not issue titles. Limit: only prose is 15; chrome is 14 and below |
| Names at weight 900 | Names at 510–590 | Authors are the product: you scan for who spoke. Limit: T2, two kinds per pane |
| Robots as avatars, with live faces | Round photo avatars | An agent says it is an agent by how it looks, and its face is its state. Limit: 36 px or less in the workspace; one size per list (Z4) |
| Status marks that move | Static state icons | "Thinking", "running", "blocked" are live facts. Limit: M4 and M7; a mark that reports nothing does not move |
| Messages arrive while you read | Updates on navigation | Limit: M3. Nothing already on screen moves; if you are scrolled up, a "new messages" line appears instead of a jump |
| State chips on messages (Starting, Finding, Submitted) | Plain comments | The chip is the message's type and drives the inbox. Limit: one chip per message, `RADIUS_SM`, `META` 12/700, the state's `-bg` and `-ink`. A DONE is the agent's claim: neutral "Submitted" until serve tracks it further, then the word Deliveries gives it (Verification failed and With fixes red, In review amber, Integrated or Installed green once git has it where it can go). Never green on the agent's word alone (C3) |
| Amber banners for a contested checkout | No equivalent | Two writers in one checkout loses work. Limit: one banner row of 32 px per alert, at most two visible, the rest behind a count |
| Season decorations | None | They are a gift, switched off in one click. Limit: they live in the frame band above the panel and in the rail's empty space; they never cross text, a control, or the panel's content, and they obey M7 |
| A Strip window with large type and a scene | None | It is read from across the room. Limit: its sizes scale together from one factor; the rules on weight, truncation and colour still hold |
| The mark with a presence dot | A static logo | The dot is the fleet's state. Limit: four states, the `mark-dot*` tokens |

### 3.1 Character surfaces

Decided by Petru on 2026-11-11, after using the native app next to the console it replaced: the
console's character was part of the product, and the native app lost it. On the surfaces below, the
reference is the console at commit `f6a79ed` (`apps/console`, its CSS and its captures in
`assets/screens/`), not Linear. Where a rule in section 2 conflicts with that reference on one of
these surfaces, the console wins. Everywhere else section 2 holds.

| Surface | What it keeps from the console |
|---|---|
| Lobby | The tinted header band and its large title, the row of robots on a shelf with their names, a card per agent with a band in the robot's tile colour and a 104 px portrait over it, a presence pill, harness and project chips, the intro as a speech bubble, lift and a short wiggle on hover, the arrival and the season's visitor |
| Agent hover card | A robot of 96 px or more with a live face, the name in display type, pills for project and state, a tinted "Now" block and tinted step rows with icon tiles, progress in a band, a red "Watch live" with a pulsing dot, an arrow to the agent and a fade-in. It stays inside the panel |
| DM header and list | A 64 px robot and display name in the header, 48 px rows, an accent pill for unread |
| Activity, History, Retro, Deliveries | Pills for "Needs you" and "Today", the current step's halo, coloured state chips, a filled primary action |
| Calm screen, Setup, away card | Their large type, the 120 px avatar, the real mark at 88 px, the staged entrance |
| Strip | Robots standing on the conveyor at full size without a tile, black weights, the season's lights and snow, animated status marks |
| Preferences and the user menu | Icons on tinted tiles, the 44 px page tile, the 56 px photo |
| Robots everywhere | A size tier per surface (36 dense lists, 48 rows, 64 headers, 96–104 cards, 120 calm), corners at 22 % of the size, a −3° tilt on hover, the smile from 40 px up |

Motion on these surfaces follows the console's durations (lift and fade 120–200 ms, wiggle 620 ms,
arrival 380 ms) and still obeys reduced motion (M7). Items 4, 5, 13, 21 and 29 of the audit in
section 6 are withdrawn by this section.

## 4. Patterns that are not allowed

Each has a way to find it and the rule that forbids it. "Capture" means a 1440×900 capture at 1×.

| Pattern | How to find it | Rule |
|---|---|---|
| Gradients | Code: `linear_gradient`. Capture: any fill whose colour changes across it | C1, C4. None, anywhere, including the Strip's panels |
| Large rounded cards | Code: `rounded(px(N))` with N above 12. Capture: a box with corners you can see from arm's length | R1 |
| Card inside a card | Capture: a bordered box inside a bordered box. Code: two nested `border_1()` with backgrounds | C4, C5. The inner one becomes plain text with space above |
| Heavy or decorative shadows | Code: `shadow(` on anything that is not a menu, popover, dialog or the finder | C6 |
| Border and shadow together | Code: `border_1()` and `shadow(` on the same element | C6 |
| Emoji in the interface | Code: a non-ASCII pictograph in a string outside user content. The status picker's presets are user content and stay | Icons are Lucide, Z3 |
| Filler copy | A subtitle that restates the title; a caption that explains a button already named for what it does; "Welcome", "Let's", "Simply" | Each screen title has at most one `SMALL` line under it, and only if it says something the title does not |
| Invented numbers | A count, percentage or duration not read from the bus or the store | Every number on screen traces to a field |
| Default blue or purple | Code: a literal blue. Capture: blue that is not `focus`, `menu-hover` or a chart series | C1, C2 |
| Pills everywhere | Code: `rounded_full()` on anything but a dot, a count badge or a round rail button | R2 |
| Outlined pills as counters | Capture: a rounded outline around "N unread" or "Needs you: N" | C2, F2. A count is a badge or plain `META` text |
| Centred hero copy | Capture: a large heading centred in a pane with a paragraph under it | F3 |
| Inconsistent spacing | Capture: left edges that do not share one line. Code: a `px(` value not in `space` | S1, S2 |
| Decorative badges and icon tiles | Capture: an icon sitting on a coloured square that carries no state | C2, C9. The icon stands alone at `icon_size::ROW` |
| Icon soup | Capture: more than one icon in a row's leading edge, or an icon before every label in a list of text | A row has at most one leading icon and one trailing mark |
| Everything bold | Code: count `FontWeight::BLACK` and `BOLD` against `NORMAL` in the file. Capture: squint; if nothing recedes, it fails | T2, T3 |
| Colour as decoration | Capture: red, amber or green on something that is not in that state | C3 |
| Repeating a fact | Capture: the same word twice in one row ("Delivered" as chip, as sentence and as step) | Say it once, in the strongest of the three places |
| Clipped text | Capture: a glyph cut in half at a container's edge | T7 |
| Explaining the interface | Captions such as "Doesn't resolve Needs you" under a button | Rename the button so it needs no caption, or move the sentence to its tooltip |
| Underlined bold links as actions | Capture: an underlined coloured label doing a button's job | Navigation in a header is a `control::SM` quiet button with a 16 px icon |
| Oversized actions | Code: `.h(px(N))` with N above 36 on a button | Z1 |

## 5. Checklist before a native UI change is called done

A change is not done until each line is true. Captures are taken from the running app, never
reasoned from the code.

1. **Captures exist** for every screen touched, at 1440×900, in dark and in light, in the default
   theme and in one other theme, with a season on and off. Each is attached to the change.
2. **States are captured:** rest, hover, keyboard focus, selected, unread, empty, and the longest
   realistic content (a 60-character name, a three-line message, a count of 99+).
3. **One left edge.** A vertical line at the pane's left inset touches the title, the first row and
   the composer (S2).
4. **Sizes measured.** Header 56. Sidebar rows 28. Controls 28, 32 or 36. Avatars one size per
   list. No measured value outside section 2.3.
5. **Type measured.** Only the sizes in 2.1. One 18/900 title per pane. At most two kinds of 900
   text per pane (T2, T4).
6. **Squint test.** At a quarter size the capture shows: the title, the selected thing, the unread
   things, and one primary action. If more than that stands out, weight or colour is overused.
7. **Accent counted.** At most three accent elements per pane (C2). No state colour without the
   state (C3).
8. **No pattern from section 4.** These searches over the files you touched return nothing new:
   - `rg "linear_gradient|rounded\(px\((1[3-9]|[2-9][0-9])" apps/desktop/src`
   - `rg "text_size\(px\(" apps/desktop/src` (every size comes from `size::`)
   - `rg "FontWeight::(SEMIBOLD|EXTRA_BOLD|MEDIUM)" apps/desktop/src`
   - `rg "hsla\(|rgb\(|#[0-9a-fA-F]{6}" apps/desktop/src/screens apps/desktop/src/shell`
   - `rg "from_millis\(" apps/desktop/src` (every duration comes from `motion::`)
9. **Every `.hover()` has a focus ring** and, if it is icon-only, a tooltip (F1, K3).
10. **Text overflow declared** on every label that holds data (T7); captured with the long content
    from step 2.
11. **Motion checked with reduced motion on:** nothing moves. With it off: nothing already on
    screen moves when new content arrives (M3, M7).
12. **Idle cost.** With no live marks visible, the window does not repaint (M4).
13. **Keyboard pass.** The screen is used start to finish without the pointer (K4).
14. **Copy.** No sentence explains the interface; every number comes from data; both languages
    fit without clipping.
15. **Shape unchanged.** The rail, the panes and their order are where they were in the capture
    from before the change.

## 6. Audit of the native screens

State of the screens at the time of writing, from captures of the running app in dark and light and
from the sources. Ranked by how much a person sees. Format: screen · what is off · rule · fix. Every
fix keeps the shape.

### Seen at first glance

1. **Every screen** · Weight is inverted: 82 Black and 129 Bold call sites against 36 Regular.
   Titles, row titles, tabs, buttons, badges, links and disclosures all shout, so nothing does ·
   T2, T3 · Apply the weight roles; row titles and buttons to 700, second lines, disclosures and
   links to 400, Black only on pane titles and names.
2. **Activity, detail pane** · The reply field's placeholder sits on the border with no padding;
   "Mark resolved" is a 48 px, 17 px-type button; "Go to channel" is an underlined bold link · S4,
   Z1, T1 · Field padding 12; buttons `control::MD` at 14/700; the link becomes a quiet header
   button.
3. **Lobby, side pane** · The composer is drawn over the amber notice: two texts overlap · S3, T7 ·
   Stack the notice above the composer with 8 px between; give the field its 12 px padding.
4. **Lobby, cards** · A 104 px initials tile, a tinted band across the card's top, and a bordered
   introduction box inside the bordered card · Z4, C9, C4 · Avatar 36 beside the name, no band, the
   introduction as plain `SMALL` text in `ink-2`, one border, `RADIUS_MD`.
5. **Activity, DMs, Lobby** · Pane titles are 24, 22 and 32 px while the detail pane beside them is
   18; the two baselines do not meet · T4 · One `TITLE` 18/900 in a 56 px header on both sides.
6. **DMs, Activity, History, Deliveries, Lobby** · Authors are saturated initials tiles at 36–48
   px while Home and Threads show robots: one agent looks like two different things · Z4, C9 · The
   same avatar component everywhere, 36 px, `RADIUS_MD`; initials only when no robot exists.
7. **Activity list** · Unread is a 20 px solid red disc, and red also means blocked on the same
   screen · F2, C3 · An 8 px `accent` dot at the leading edge; counts in the 16 px badge.
8. **Home** · Two full-width amber banners take 80 px above the messages and are the brightest
   thing on the screen · C3, section 3 · 32 px rows, `SMALL` 13, `warn-bg`/`warn-ink`, the lead-in
   at 700 and the rest at 400; from the third alert on, a count.
9. **DMs and Activity rows** · The second line is cut mid-glyph at the pane's edge ("…project
   histor") · T7 · `.line_clamp(2)` on previews, `.truncate()` on single lines.
10. **Frame, with a season on** · The string of lights runs across the top edge of the panel and
    snow dots land on the rail's labels ("Activity", "Search") · Section 3, decorations · Keep the
    lights inside the frame band above the panel; exclude the rail's label and icon boxes from
    where snow can be drawn.

### Seen within a minute

11. **Activity and DMs headers** · An outlined accent pill ("Needs you: 4", "8 unread") repeats the
    tab's and the rail's count · C2, F2, section 4 · Remove the outline; if the count stays, it is
    `META` text in `ink-3`.
12. **Deliveries** · Each row says "Delivered" three times (chip, "delivered yesterday at…", the
    first step) and marks the current step with a red-toned dot that reads as blocked · C3, section 4 · Drop the chip where the
    steps are shown; steps done in `green`, current in `ink`, pending as a `line-strong` ring.
13. **Preferences** · Every nav item has an icon on a pink tile, the page header has a 44 px
    accent tile, and the selected row is accent-filled around a dark tile · C2, C9 · Icons alone at
    16 px in `ink-3`; the page title is text; keep the selected row's fill.
14. **Home, channel header** · "N sessions today" is set in accent and "N working now" in green,
    both bold · C2, C3 · The count in `ink-3` at 400; only "working now" keeps `green-ink`.
15. **Home, messages** · "Technical details" is 13/700 under every message and the reply count is
    accent 700: the secondary lines compete with the message · T2, C2 · Disclosure at 13/400 in
    `ink-3`; reply count in `link` at 400.
16. **Sidebar** · Section labels are capitals without tracking; rows are 32 px at 15 px; the agent
    filter and rows are tighter than the channel rows · T5, C7 · Sentence-case labels at 12/700,
    rows 28 px at 14/400, agent rows 44 px.
17. **Sidebar, light mode** · The warning icon on the selected channel is red on the accent fill
    and nearly disappears · C8 · On a selected row every mark uses `accent-ink`.
18. **History** · Rows are 70 or 94 px depending on content, the state is an outlined pill at the
    far right, and the full path is shown in mono · Z2, R2, T6 · A fixed two-line row with an
    optional third; the state as a `RADIUS_SM` chip beside the name; the path shortened from the
    left with `.truncate()`.
19. **Activity, list header** · "Mark all as read" and "Resolve all" are accent bold text with a
    caption explaining what one of them does not do · C2, section 4 · Two quiet `control::SM`
    buttons in `ink-2`; the caption becomes the tooltip.
20. **Empty states** · "Pick an alert to see it in context" is one line at the top; "Choose an
    agent" is a title and a sentence; the Lobby's is a sentence in the middle · F3 · One block, one
    position, on all three.

### Seen when you look for it

21. **Strip** · Names, labels and the clock are all heavy; previews and file names are cut at the
    card edge; initials tiles are 96 px · T2, T7 · Names 700, status 400, `.truncate()`; robots at
    the scaled avatar size.
22. **Every screen** · 68 hover sites, 21 focus sites, 9 tooltips: most of the app cannot be seen
    under the keyboard and no shortcut is shown anywhere · F1, K2, K3 · Focus ring wherever there
    is hover; tooltips with keys on icon buttons; keys at the trailing edge of finder rows.
23. **Every screen** · 19 radii in the sources (2 to 35 px) · R1 · Five.
24. **Every screen** · 30 padding values and 22 margins, among them 5, 7, 9, 11, 14, 18, 22 and 26
    · S1 · The nine steps.
25. **Every screen** · 19 animation durations from 33 ms to 2.5 s with no shared easing · M1 ·
    `motion::` constants; loops keep their periods and stop when hidden.
26. **Count badges** · 18 px high at weight 900: the heaviest text on the screen is a number ·
    T2, F2 · 16 px, 11/700.
27. **Every screen** · About 280 literal `text_size(px(…))` calls against 72 uses of the `size::`
    constants: the scale exists and is mostly bypassed · T1 · Every size through `ui::text()` and
    `size::`.
28. **Dialogs (profile, update, status)** · Titles at 18, 20, 21 and 22 px and headers of 56 and
    64 px · T4 · `TITLE` in a 56 px header on all of them.
29. **Setup and the calm screen** · Display sizes of 34 and 50 px and 72 px buttons · T1, Z1 ·
    `DISPLAY` 24 for the heading, `TITLE` below it, `control::LG` for the actions.
30. **Tabs (Activity)** · The selected tab is heavy with a 2 px accent underline, and a filled count sits
    beside an unselected label · T2, F2 · Selected 700 in `ink`, others 400 in `ink-3`; the count as
    the 16 px badge.

## What could not be sourced

- The signed-in Linear app's stylesheet. Component sizes come from the app replica on the public
  homepage; they agree with what is seen in the product, but they are the replica's values.
- Sizes and weights of Inter Display in the app. Linear documents that it is used for headings and
  nothing more.
- "No spinners" and optimistic writes as Linear's stated rule. They are visible in the product and
  described by third parties (for example
  <https://performance.dev/what-makes-linear-feel-instant>), not in a Linear design post. M5 and M6
  stand on harbard's own architecture: the bus is read in-process.
- The shortcuts overlay and tooltips showing keys: seen in the product; no public page documents
  how they are drawn. The key-chip size in K1 is measured from the homepage's command menu.
- Brand colour values: the brand page describes the colour in words only. This document copies no
  Linear colour.
