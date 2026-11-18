# Decluttering the screens: Play / Set up with an inspector

Status: **agreed direction, not built yet** (decided 19 Nov 2026).

The chosen design mixes the two mockups: Option A's separate **Play** and
**Set up** modes, plus Option B's **inspector** (a settings panel for whatever
you selected) and folding sections on the Set up pages.

## Why

What makes Kickline feel cluttered today (`before_main.png`, `before_output.png`):

- Every layer shows all its controls at once: Bypass, stop, remove, Opacity,
  Blend, Reacts to, Next clip on, Effects.
- Advanced Output, Fixtures, Audio, DMX Monitor and Diagnostics are floating
  windows that land on top of the clips.
- The palette takes two rows (presets, then the editor).
- The top bar is so full it shortens its own labels on narrow screens.
- Advanced Output stacks lumiverse settings, node status, sweep and the fixture
  list in one long column, and two rows of number boxes under a small canvas.

Kickline's main job is a DJ playing in a dark room. A Play mode that can't be
broken mid-set is worth more than having set-up one click closer.

## The design

### Top bar (both modes)

BPM · Link beat dots · Master · **Blackout** · output badge
(OUTPUT LIVE / DEAD with universe count) · **Play | Set up** switch · show menu.

The output badge moves up from the status bar, so the bottom strip can go.

### Play (`a_play.png`)

Only what's needed during a set:

- **Palette** in one row: presets, Next, and the current "changes on…"
  setting as a short note. Editing a palette moves to the inspector / ⋯.
- **Clip grid** with bigger tiles; column triggers above it.
- **Each layer** is just: colour strip, name, opacity fader, **M** (mute) and a
  one-word hint of what it reacts to ("♪ Kick"). Everything else is behind **⋯**.
- **Right side:** "What the lights get" preview; Music card (source, live
  kick / snare / hat dots); Lights card (each output with its live / answering dot).
- Clicking ⋯ on a layer, or a clip's settings, opens the **inspector** as a
  slide-in panel on the right instead of a popup (from Option B), and closes
  again, so Play stays clean by default.
- Nothing in Play can move fixtures or change outputs.

### Set up (`a_setup.png`)

Full pages, not floating windows. Left navigation:
**Output · Fixture library · Music · DMX values · Diagnostics**.

**Output page:**

- Left column: **Outputs** (each lumiverse as a card: name, status dot,
  universes and IP; Find… button; + Add output), then **Fixtures** (name and
  address; + Add).
- Middle: a big canvas, with "Show what the lights get" and Snap.
- Right: the **inspector** for the selection:
  - a fixture: name and type line, **Test this light**, Edit type, **Live
    channels**, then folding sections: *Address*, *Position & size*,
    *Wiring & colour*;
  - an output: name, protocol, IP, network port, then folding sections:
    *Sync & refresh*, *Sweep universes*, *Node status*.
- The setup guide, Add array and Sweep open as steps in the inspector
  rather than extra panels.

**Other pages** follow the same pattern: list on the left, main view in the
middle, inspector on the right. Fixture library: search and results / pinout
/ details. Music: source and meters / drum sensitivity. DMX values: the
current monitor. Diagnostics: checks and report.

### From Option B, kept

- The inspector pattern (one place for "settings of the thing I clicked").
- Folding sections, so the most-used controls are first and the rest is one
  click away.

### From Option B, not used

- The always-visible bottom dock: it takes room from the clips during a set.

## Notes for building

- Keep every existing feature reachable; this is layout, not removal.
- Keyboard: Tab (or ⌘1 / ⌘2) switches Play / Set up; Esc closes the inspector.
- Selecting a fixture on the Set up canvas and switching to Play keeps the
  selection for later.
- Existing window state (which windows were open) maps to the Set up page
  that was last used.
- The status bar's extra info (rate, net/sub-net, ranges) moves to the
  output badge's tooltip and the Lights card.
- Estimated at about two sessions.

## Files

- `before_main.png`, `before_output.png`: the app as of v0.10.0.
- `a_play.png`, `a_setup.png`: Option A, the base of the chosen design.
- `b_inspector.png`: Option B, for the inspector and folding sections.
- `mockup-src/`: the HTML generator for the mockups (`python3 make.py`, then
  render with a headless browser at 1280×800).
