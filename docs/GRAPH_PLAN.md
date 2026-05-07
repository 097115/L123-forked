# Graph Settings & /Graph implementation plan

Companion to `docs/SPEC.md`, `docs/MENU.md`, and `docs/PLAN.md` M7.
Execution plan for closing the gap between what 1-2-3 R3.4a does when
the user invokes `/Graph` and what L123 does today.

Source of truth for menu shape and field semantics:
*Lotus 1-2-3 R3.1 Reference*, pp. 2-154 → 2-230. Excerpts inlined where
the wording matters; do not duplicate the manual.

When code and this doc disagree, fix the doc first, then write code to
match. Same rule as `CLAUDE.md`.

---

## 0. The visible gap

In R3.4a, pressing `/G` overlays a **Graph Settings** sheet over the
worksheet — a four-panel readout of the current graph. The user
navigates the menu line and the panels redraw to reflect every setting
change. See *Reference* p. 2-230 (figure embedded in `/Graph` reference).

In L123 today, pressing `/G`:

- shows the menu line (`Type X A B C D E F Reset View Save Options Name Group Quit`),
- but the worksheet stays visible underneath; **no settings sheet at
  all**.

That is the user-facing symptom. Underneath, three things are missing:

1. **Data model** — `GraphDef` (`crates/l123-graph/src/lib.rs:55`) holds
   only `graph_type`, `x`, and `data[6]`. Roughly two-thirds of the
   fields the settings sheet displays (Y/2Y assignments, frame mask,
   stack/percent/3-D/drop-shadow/table flags, all of `/Graph Options`)
   have no place to live.
2. **Menu wiring** — `crates/l123-menu/src/lib.rs:3503-3703` covers
   `Type {seven types}`, `X`, `A-F`, `Reset Graph`, `View`, `Save`,
   `Quit`. Missing: `Type Features`, all of `Options`, `Name`, `Group`,
   and the deeper `Reset` leaves.
3. **Render** — `crates/l123-ui/src/app/render.rs:504` only branches on
   `Mode::Graph` (full-screen view, F10). There is no `Mode::GraphMenu`,
   no settings overlay function.

---

## 1. Architecture pattern (read first)

Every slice below walks the same five layers in this order:

```
1.  l123-graph              — define data types (no engine deps)
2.  l123-menu               — declare Action enum + menu tree leaves
3.  l123-ui types.rs        — store live state on Workbook / App
4.  l123-ui render.rs       — render the settings overlay panels
5.  l123-ui keys.rs / cmd   — handle Action dispatch
6.  tests/acceptance        — keystroke transcript proves it
```

The closest existing reference shape to copy is the **Global Default
Settings** overlay:

- `render_defaults_overlay` at `crates/l123-ui/src/app/render.rs:873` —
  one outer block, sub-blocks per panel, `Paragraph` body fed from a
  single typed struct on `App`.
- `Mode::Stat` + `StatView::Defaults` enum pair (`types.rs:255`,
  `mod.rs:342`) — the toggle that flips render between worksheet and
  overlay.
- `defaults: GlobalDefaults` field on `App` — single source of truth
  for what the panel reads.

We model `Mode::GraphMenu` the same way, with a single `GraphDef` that
the panels project read-only.

### TDD discipline

Per `CLAUDE.md`:

1. Acceptance transcript written first (red).
2. Unit test on the data-type or render helper (red).
3. Minimum code to green.
4. Refactor with green tests; clippy `-D warnings` clean.

Every menu leaf that mutates the graph gets ≥1 acceptance transcript
under `tests/acceptance/graph_<feature>.tsv`. Visual diff is asserted
via `ASSERT_SCREEN <substr>` against the settings panel.

### Layering rules to honor

- `l123-graph` stays zero-engine-deps (it imports `l123-core::Range`
  only). New types — `GraphFeatures`, `GraphOptions`, `Frame`, `YAxis`,
  `LineFormat`, `ScaleAxis`, etc. — live alongside `GraphDef` in
  `crates/l123-graph/src/lib.rs` (or a sibling `model.rs` if it grows
  past ~200 lines).
- The settings overlay is **read-only projection**. Mutation lives in
  `app/mod.rs` (or a new `app/cmd/graph.rs` if `mod.rs` grows further —
  see `CLAUDE.md` §"app submodule layout"). The overlay never owns
  state.

---

## 2. Slices

The work is too big to land in one PR. Six slices, each landable
independently. Slice A and B are blocking for the visible gap; the rest
backfill behavior the panel already exposes.

### Slice A — Data model (`l123-graph`)

Grow `GraphDef` (or wrap it in a parent `Graph` struct) so every field
the settings sheet displays has a home. The shape, derived from the
Reference command tree on p. 2-154 and the `Graph Settings` figure on
p. 2-230:

```rust
pub struct Graph {
    pub graph_type: GraphType,
    pub x: Option<Range>,
    pub data: [Option<Range>; 6],
    pub features: GraphFeatures,
    pub options:  GraphOptions,
}

pub struct GraphFeatures {           // /Graph Type Features
    pub orientation: Orientation,    // Vertical (default) | Horizontal
    pub stacked:     bool,           // Stacked Yes/No
    pub percent:     bool,           // 100% Yes/No
    pub y_axis:      [YAxis; 6],     // per A-F: First (default) | Second
    pub frame:       FrameMask,      // Left/Right/Top/Bottom flags
    pub drop_shadow: bool,
    pub three_d:     bool,
    pub table:       bool,
}

pub struct GraphOptions {            // /Graph Options
    pub legend:      [Option<String>; 6],
    pub format:      [LineFormat; 6],   // Lines/Symbols/Both/Neither/Area
    pub titles:      Titles,            // first/second/x/y/2y/note/other-note
    pub grid:        GridMask,          // Horizontal/Vertical/Y-Axis flags
    pub scale_y:     ScaleAxis,
    pub scale_x:     ScaleAxis,
    pub scale_2y:    ScaleAxis,
    pub skip:        u32,               // /Graph Options Scale Skip
    pub color:       bool,              // Color (true) vs B&W (false)
    pub data_labels: [Option<Range>; 6],
    pub advanced:    Advanced,          // colors / hatches / text — flesh out in slice F
}
```

Notes:

- Defaults match the *Reference* (Vertical orientation, no stack/percent,
  Y-Axis = First for all six, no frame, color on, no grid, automatic
  scale on every axis, data-labels unset).
- `FrameMask` and `GridMask` use named bool fields, not bitflags — the
  manual lists each side individually and the panel renders one `x` per
  side.
- All fields `Default + PartialEq + Eq + Clone + Debug`. `Eq` is fine
  because everything reduces to enums, bools, ints, strings, and ranges.
- Persistence: `NamedGraphs = BTreeMap<String, Graph>` keeps the same
  shape. Workbook serde rolls forward; absent fields read as `Default`.

**Tests:**

- Default round-trip: every leaf documented in `MENU.md` § "/Graph"
  starts in its Reference-default state.
- `Graph::reset()` (== `/Graph Reset Graph`) returns to default.
- `reset_options()` / `reset_ranges()` clear only that subtree.

**No UI changes in this slice.** `app::Workbook` keeps a `Graph` instead
of a `GraphDef`; existing X/A-F mutations pass through. The diff is
mechanical; existing tests should stay green.

### Slice B — Graph Settings overlay (`l123-ui`)

Smallest visible change. New `Mode::GraphMenu` (distinct from
`Mode::Graph`, which is the F10 full-screen view). Entered from READY
by `/G`; left by `Quit` or Esc back to READY.

While in `GraphMenu`:

- Top: control panel + menu line + submenu line (already drawn by the
  existing menu code).
- Body: **Graph Settings** overlay, four panels matching the screenshot:

```
┌─────── Graph Settings ───────────────────────────────────────────────┐
│ ┌Graph Type─┐  ┌Graph Type Features─┐  ┌Options──────────┐           │
│ │   Line  x Pie  │  │ Y/2Y    x Vertical │  │ x Colors on   │           │
│ │   Bar     HLCO │  │  A: Y     Horizontal│  │               │           │
│ │   XY      Mixed│  │  B: Y               │  │ Grid Lines    │           │
│ │ Stacked Bar    │  │  C: Y    Frame      │  │   Horizontal  │           │
│ └────────────────┘  │  D: Y   x Left      │  │   Vertical    │           │
│                     │  E: Y   x Right     │  │ x Y-Axis      │           │
│ ┌Data Ranges──────┐ │  F: Y   x Top       │  │   2Y-Axis     │           │
│ │ X: A:A9..A:A13  │ │         x Bottom    │  └───────────────┘           │
│ │ A: A:B9..A:B13  │ │                     │                              │
│ │ B:              │ │ Stack data ranges   │                              │
│ │ C:              │ │ Percentage          │                              │
│ │ D:              │ │ Drop-shadow         │                              │
│ │ E:              │ │ 3-D                 │                              │
│ │ F:              │ │ Table               │                              │
│ └─────────────────┘ └─────────────────────┘                              │
└──────────────────────────────────────────────────────────────────────┘
```

Each `x` is a green-on-black highlight when the corresponding flag is
on; rendered as a literal `x ` prefix when off it's two spaces.

Implementation:

- New free fn `render_graph_settings_overlay(graph: &Graph, area, buf)`
  in `crates/l123-ui/src/app/render.rs` near `render_defaults_overlay`.
- New `app/render.rs` branch in `render` (next to the existing
  `Mode::Graph` arm at line 504): `else if self.mode == Mode::GraphMenu`.
- Keystroke entry: when the menu enters the `/Graph` root, set
  `self.mode = Mode::GraphMenu`. On `Quit`/Esc back to READY restore
  the mode.

**Tests:**

- Acceptance `tests/acceptance/graph_settings_visible.tsv` — `/G` →
  `ASSERT_SCREEN "Graph Settings"`, `ASSERT_SCREEN "Graph Type"`,
  `ASSERT_SCREEN "Data Ranges"`, `ASSERT_SCREEN "Graph Type Features"`,
  `ASSERT_SCREEN "Options"`. Esc → `ASSERT_MODE READY`.
- Unit test: render to a 80×25 `Buffer`, assert panel borders land
  inside the grid region.

This slice is the user's immediate ask. Mutations in slices C–F
re-render this overlay automatically because it is a pure projection.

### Slice C — `/Graph Type` + `Type Features` wiring

Adds the menu leaves that mutate `GraphFeatures`. Per Reference
pp. 2-194 → 2-198:

- `Type Features Vertical` / `Horizontal` → set `orientation`.
- `Type Features Stacked Yes|No` → toggle `stacked`. Disabled (gray, no
  effect) for Pie and HLCO.
- `Type Features 100% Yes|No` → toggle `percent`. Disabled for Pie/HLCO.
- `Type Features 2Y-Ranges {Graph|A-F|Quit}` → set `y_axis[i] = Second`.
- `Type Features Y-Ranges  {Graph|A-F|Quit}` → set `y_axis[i] = First`.
- `Type Features Frame {Left|Right|Top|Bottom|All|Clear|Y-Axis} Yes|No`.
- `Type Features Drop-Shadow Yes|No`.
- `Type Features 3-D Yes|No`.
- `Type Features Table Yes|No`.

**Tests:** one acceptance per leaf (`graph_features_orientation.tsv`,
`graph_features_stacked.tsv`, …). Each presses `/GTF…`, asserts panel
shows the new state, then asserts the field flipped via a unit hook
(`app.workbook().current_graph.features.orientation == …`).

### Slice D — `/Graph Options` wiring

Adds Legend, Format, Titles, Grid, Scale, Color/B&W, Data-Labels.
Reference pp. 2-200 → 2-218.

This slice is the largest leaf-count and benefits from being done in
sub-slices:

- D1 — Legend, Color/B&W (smallest, exercises pattern)
- D2 — Titles (first/second/x/y/2y/note/other-note, all string prompts)
- D3 — Grid, Format (per-series enum)
- D4 — Scale (per-axis sub-tree: Automatic/Manual/Lower/Upper/Format/
  Indicator/Type/Exponent/Width)
- D5 — Data-Labels (per-series range + position enum; Reference
  p. 2-204)

`Advanced` (Colors / Hatches / Text → font/size) is **deferred to slice F**
because it only affects raster rendering, not the settings panel.

### Slice E — `/Graph Name` and `/Graph Group`

`/Graph Name {Use|Create|Delete|Reset|Table}` — the storage layer
(`NamedGraphs`) already exists; only menu leaves and the F3-pop list
are missing. Reference pp. 2-218 → 2-224.

`/Graph Group {Columnwise|Rowwise}` — auto-graph helper. Reads default
orientation from `GlobalDefaults::graph_group` (already wired). Walks
the user-supplied range, fills X and A-F by columns or rows.
Reference p. 2-172.

### Slice F — Render the new attributes in the F10 view

Until this slice the settings panel shows fields the F10 unicode
renderer ignores. After:

- `orientation = Horizontal` rotates Bar 90°.
- `stacked` and `percent` change Bar/Stack behavior.
- `frame` mask draws box edges in `l123_graph::render`.
- `grid` mask draws axis grid lines.
- `titles` (first/second/x-axis/y-axis) overlay above and along axes.
- `legend` row appears below the plot.
- `three_d` adds depth shading on Bar/Pie/Line (raster path only —
  the unicode path documents the limitation in the placeholder text).
- `table` displays a small text table under the chart.
- `Advanced` colors/hatches/fonts → SVG/PNG output only.

Same renderers, expanded. The unicode path stays minimal; the raster
path (`crates/l123-graph/src/raster.rs`) gets the lion's share.

---

## 3. Acceptance gate (Authenticity Contract addition)

Add to SPEC §20 once Slice B lands:

> Pressing `/G` from READY overlays the four-panel **Graph Settings**
> sheet (Graph Type, Data Ranges, Graph Type Features, Options) and
> stays on screen for the duration of the `/Graph` menu. Esc / Quit
> dismisses it back to READY.

Each later slice adds one bullet:

> `/G T B` shows `x Bar` in the Graph Type panel.
> `/G T F H` shows `x Horizontal` and clears `x Vertical` in Type Features.
> `/G O G H` shows `Horizontal` lit under Grid Lines.
> …and so on.

---

## 4. Out of scope (named, so future-us doesn't chase)

- **WYSIWYG `:Graph` menu** — colon-prefix path that embeds graphs in
  the worksheet visual. Tracked separately under `:` MENU.md leaves.
- **CGM/PIC export fidelity** — `/Graph Save` already writes SVG;
  binary CGM and PIC are deferred to a separate PLAN.md milestone.
- **Plotter / printer driver matrix** — `/Graph Options Advanced` plays
  back through the print pipeline (`l123-print`); the matrix lives
  there, not here.
- **Live `/Worksheet Window Graph` panel** — the in-window companion to
  the Settings sheet. Listed under `MENU.md` `/Worksheet Window Graph`;
  builds on slices A, B, F but ships in its own PR.

---

## 5. Cross-references

- `docs/SPEC.md` §M7 (Graphs) and §20 (Authenticity Contract).
- `docs/MENU.md` "/Graph" section — keep this doc and MENU.md in sync;
  if the manual disagrees, fix MENU.md first.
- `docs/PLAN.md` "M7 — Graphs (week 12-14)" — high-level milestone.
  This doc decomposes that milestone.
- *Lotus 1-2-3 R3.1 Reference*, pp. 2-154 → 2-230 — canonical menu and
  field definitions. PDF lives at `~/Library/Mobile Documents/com~apple~CloudDocs/lotus123/Lotus 1-2-3 3.1 Manuals (1990)/Lotus 1-2-3 Release 3.1 - Reference.pdf`.
