# Auto VJ: Kickline runs the whole set by itself

Status: **plan only** (27 Nov 2026). Nothing here is built yet, and nothing
should be built until the user says to start.

This document is written for the model (or person) who will build it. It
says what is wanted and why, what was decided, and what is left to you.
Read `README.md`, `ROADMAP.md` and the module docs at the top of
`crates/lumen-core/src/director.rs`, `sections.rs`, `scenes.rs` and
`crates/lumen-app/src/engine.rs` before you start.

## Intent

Kickline is one DJ's tool for his own LED rig. He wants to start rekordbox,
connect it, and have Kickline light the whole night the way a good VJ
would, without touching it:

- It hears the music correctly: tempo, the beat, the "1" of the bar, and
  which part of the song is playing.
- Each part of a song gets its own kind of effect, by rule. Chases belong
  to a build-up, flashes to a drop.
- It doesn't repeat itself. The second drop looks a little different from
  the first, and the same for every other part.
- It lights the stage as the shape it is. The stage changes from show to
  show: a dragon one night, a face the next, then a body.
- It never falls over mid-set.

## What is wrong today

1. **Auto is dice.** `Director::choose` picks from one shared bank of 26
   effects by weighted random (energy fit, section tag, "not one of the
   last three"). A chase and a strobe are both allowed in a drop. Nothing
   remembers what the last drop looked like.
2. **Too few effects.** `scenes::tags_for` gives each section only a
   handful of effects that really suit it. Ten distinct looks per section
   cannot be made by reordering what exists.
3. **Sections are misread with Ableton Link on.** Reported from real use;
   cause **not known**. What the code does: with Link on and a peer
   present, `Engine::tick` calls `comp.set_clock(link_beat, link_tempo)`,
   and `SectionDetector::update` cuts the audio into beats at
   `beat.floor()` of that clock. So the detector trusts Link completely
   for where each beat starts. The user plays with LINK on for every
   deck, so a free-running deck is probably not the cause. Things to
   check against a recording, in this order: Link's beat jumping or
   restarting when a deck is loaded or a peer joins; the "fire early"
   lookahead and the capture delay shifting kicks into the wrong beat;
   rekordbox beat grids that are off for some tracks; the peer count
   dropping to zero so the clock falls back to free-running; and the
   drum detection itself behaving differently on the real feed than on
   the demo beat. Do not fix by guessing: reproduce it first (Phase 0).
4. **No tempo from the audio.** Without Link the clock is the typed or
   tapped BPM. There is no beat tracker.
5. **The stage is only rectangles and names.** A group is a name; its
   fixtures are ordered left to right (`AdvancedOutput::group_members`).
   Nothing says "these are the eyes" or "this chain runs tail to head".
6. **Not proven solid.** CI publishes a release even when tests fail
   (`continue-on-error: true` in `.github/workflows/macos.yml`). There is
   no long-run test and no automatic start-up for a show.

## Goals

- G1. Tempo, beat phase and bar position come from the audio, with Link
  and tap as helpers. Sections are right with Link on, off, or wrong.
- G2. Auto follows a **playbook**: for each of the five sections, about
  ten looks of the right family, played in rotation with no immediate
  repeats.
- G3. Looks are written for **stage roles**, so one playbook works on any
  stage once its parts are tagged. The stage changes from show to show
  and there is no "usual rig": a new stage must go from nothing to a
  running show in about ten minutes, and switching stage must not lose
  the user's playbook choices.
- G4. A stage can add a few **signature looks** of its own.
- G5. **Show mode**: open the app and it finds rekordbox, starts output
  and Auto, and recovers by itself from anything that drops out.
- G6. Every claim above is checked by a test that runs without a Mac
  window, a rig, or live audio.

## Non-goals

Windows and Linux builds, Developer ID signing, MIDI, OSC, cue lists,
moving heads, video clips, Pro DJ Link, a plugin API, and a wgpu
compositor. They stay on `ROADMAP.md`; do not start them here.

Kickline does not guess the theme from the music. The theme is the stage
shape, and the user tags it.

## Anti-goals (a failure even if it "works")

- Auto still looks random, or two drops in a row look the same.
- Anything strobes when flash-safe is on. (The user wants real strobes
  in drops, so his shows run with flash-safe off. The switch stays, for
  venues that ask for it.)
- The user has to touch the app mid-set to keep it looking right.
- Link becomes required, or a bad Link session can still wreck the beat.
- A show or stage profile saved by v0.12 no longer opens.
- A look renders nothing because the stage lacks the role it was written
  for.

## Constraints

- Keep the crate layout. `lumen-core` stays free of I/O and GUI.
- Shows and stage profiles are JSON through serde. Every new field needs
  `#[serde(default)]`; add a test that loads a v0.12 show.
- The engine thread must keep its fixed rate. Budget: one tick (director,
  render, map, send) under 5 ms on the example stage in a release build.
  Add a test or bench that measures it.
- macOS only for now. The builder has no Mac window, no rig and no
  rekordbox, so each phase ends with a short checklist for the user to run
  on the real rig.
- Keep what works in Auto: A/B layer pairs and crossfades, Hold, stepping
  back under the user's own layers, genre pace, flash-safe, and
  `same_seed_same_show` determinism.
- British spelling and plain words in UI text, as in the rest of the app.

## Approaches considered

**1. Parts with roles (chosen).** Each group gets a role from a short
fixed list. The playbook is written against roles. A new stage needs a
few minutes of tagging and then the whole playbook works on it. Cost: the
looks are generic unless a stage adds signature looks.

**2. A hand-built show per stage.** Best-looking for one stage, but hours
of work for each new one, and it is not automation.

**3. Guess the parts from the layout.** No set-up, but geometry cannot
know that two small rings are eyes.

The user chose 1 on 27 Nov 2026 because the stage changes from show to
show. Two ideas are borrowed: Kickline suggests roles from the layout and
the user corrects them (from 3), and a stage may carry a few signature
looks (from 2).

## The design

### A. Hearing the music

**Two clocks, not one.** Today one beat value does two jobs. Split them:

- the *analysis clock* is aligned to the audio as it arrives. The section
  detector and the playbook use it.
- the *render clock* is the analysis clock moved earlier by the output
  lookahead. Generators and the Beat/Bar pulses use it.

**A beat tracker** in `lumen-audio` (new module). Input: the onsets and
band levels the analyser already produces. Output: tempo, beat phase, bar
phase, and a confidence for each. It must keep counting through a
breakdown with no kicks and re-lock when they return.

**One place decides the clock** (the engine). Sources are the tracker,
Link and tap. Decided rules:

- Phase comes from the audio whenever the tracker is confident.
- Link's tempo is used when it agrees with the tracker (allowing half and
  double), and to carry the tempo through a breakdown.
- If Link and the audio disagree for more than a few bars, the audio wins
  and the UI says so in plain words.
- With no confident source, the clock free-runs on the last tempo, and
  Auto uses only looks that don't need the beat.

**The "1" and the phrase.** Dance music moves in 8- and 16-bar phrases.
The tracker keeps a bar count inside the phrase, corrected when a drop or
a big change lands. The playbook uses it to change looks on phrase lines
and to know how far through a build-up it is.

**Set memory.** Count the drops and builds heard so far, and notice a new
track (tempo or key energy shifts across a mix). The playbook reads these.

**The section detector** keeps its five sections and its tuning per
genre. It moves to the analysis clock. Whether its rules need changing is
decided from the recordings in Phase 0, not in advance.

### B. The playbook

Three new ideas, all in `lumen-core`:

- A **move** is one effect aimed at one stage role, with its settings:
  the generator, the role, direction, speed, colour slot, level and blend.
- A **look** is a few moves that play together, plus the section family
  it belongs to and whether it strobes.
- The **playbook** holds an ordered list of about ten looks per section.

How Auto chooses:

- On entering a section it takes the next look in that section's list.
  Each list is a shuffled bag: every look is used once before any
  repeats, and a new bag never starts with the look just played.
- The shuffle is seeded. Same seed and same music give the same show.
- In a long section it moves to the next look on a phrase line.
- A look can **ramp**: a setting driven by how far through the section
  the music is. This is how a build-up chase speeds up and brightens.
- A drop starts with a one-bar **impact** before its look settles.
- The pause is near-dark, with at most one small element left on.

Families (the rule the user asked for):

| Section | Family |
|---|---|
| Groove | steady motion on the beat: sweeps, meteors, dashes, alternates |
| Melodic | slow and soft: breathing, slow washes, twinkle |
| Build-up | chases and fills that speed up and tighten |
| Pause | dark, one focal element at most |
| Drop | real strobes, flashes and bursts on the kick, and effects driven by the bass; the whole rig, high contrast |

The user asked for two kinds of drop: real strobes, and effects that
move with the bass (brightness, size or reach following the bass level,
not only a flash on each kick). Both kinds must be in the Drop list, and
bass-driven looks belong in Groove too.

Ten looks per section do not need ten new generators each. Variety comes
from which role leads, direction, speed, mirror, stagger and colour slot.
Still, new effects are needed; see D.

The built-in playbook is compiled in. The show file stores only the
user's changes: looks switched off, a changed order, and signature looks.
`DirectorSettings::off_effects` from v0.12 must still load.

Colour: palette choice stays as it is (by energy, on section changes),
but a look names palette slots, never fixed colours.

### C. Stage roles

Roles (decided): **Outline, Eyes (focal points), Path, Left/Right pair,
Centre, Fill.**

- A role is set per group, in `lumen-fixtures`, saved with the show and
  with the stage profile.
- A **Path** has an order and a direction: a list of fixtures from start
  to end, each with its own "runs backwards" flag. An effect on a Path
  travels along the whole chain as one line, not along each strip
  separately. That needs a position-along-the-path value at output time;
  today `StripPixel` only knows the position within one fixture.
- A **pair** links a left group and a right group so effects can mirror
  or alternate between them.
- **Fallbacks.** Every role has a fallback chain ending in "every
  fixture", so any look renders on any stage. Test this on a stage with
  no roles at all.
- **Suggestions.** Set up → Stage offers a guess from the layout (longest
  end-to-end chain is a Path, a small matched pair is Eyes, fixtures on
  the hull are the Outline, panels and matrices are Fill). The user
  corrects it. The guess is a starting point and is never applied
  silently.
- **Signature looks** live in the stage profile and are added to a
  section's list for that stage only.
- **Changing stage.** A stage is one stage profile file. Loading another
  profile swaps the rig, groups, roles and signature looks in one step
  and leaves the playbook choices (looks switched off, order, Auto
  settings) alone. Today a profile also carries palettes and looks;
  decide what moves with the stage and what stays with the show, and
  write it down.
- **A new stage, start to finish:** setup guide (controller, lights,
  test) → place fixtures → accept or correct the suggested roles → save
  the profile → Auto runs. Count the steps and keep them few.
- The example stage is only an example; the user's real stages differ
  every time. It gets roles, and two more example stages are added (a
  face and a dragon). Tests must also run the playbook over made-up
  stages (random layouts, with and without roles) to prove every look
  lights something on any of them.

### D. New effects

Build only what the playbook needs to reach about ten looks per section.
Expected gaps:

- one-shot effects (play once from the trigger, then stop); the roadmap
  already lists per-clip one-shot transport
- breathe (slow rise and fall)
- chase and fill with speed taken from the ramp
- impact (a hit that decays over a bar)
- burst from the centre outward across fixtures
- eye glow and blink for focal points
- a single meteor or "fire" along a whole Path
- patterns for panels and matrices (checker swap, scan line)
- bass-driven effects: a throb whose brightness follows the bass, a
  shape that grows from the Centre with the bass level, a bass meter
  along a Path

Every flashing effect declares its flash rate. With flash-safe on,
nothing above 3 flashes a second plays; with it off, real strobes do.

### E. Rock solid

- **Tests gate releases.** Remove `continue-on-error` from the test step.
- **Replay tests.** Recorded sessions (Phase 0) run through analyser,
  tracker, sections and playbook in a plain test.
- **Soak test.** Run the engine headless for the length of a long set at
  faster than real time: no panic, steady memory, tick time in budget.
- **Fail soft.** Audio goes silent or the feed stalls: fade to a calm
  idle look, never freeze or go black, and return when the music does. An
  engine panic restarts the engine and logs it.
- **Show mode.** One switch, saved with the show. On launch: open the
  last show, wait for rekordbox and capture it, turn output and Auto on.
  Keep retrying each of these for the whole night.
- **Before-the-show check.** One button that runs the existing
  diagnostics and answers ready or not, with the one thing to fix.

## Phases

Each phase ends with the app releasable and a checklist for the user.

**Phase 0: Evidence.** Add "Record a session" to Diagnostics: it saves
the captured audio, the Link beat, tempo and peer count per tick, and
lets the user mark the real section with the keys 1 to 5 as it plays. Add
the replay harness. Gate releases on tests. *Done when:* a recording made
by the user with Link on reproduces the misread sections in a test.
*Needs the user:* three or four recordings of a few minutes each: the
same two or three tracks played once with Link on and once with Link
off, including at least one where the lights were clearly wrong.

**Phase 1: The clock.** Beat tracker, the two clocks, the clock rules,
phrase position, set memory. *Done when:* on the recordings and on the
demo track, tempo is within 1%, the beat is within 30 ms, the clock
holds through a 16-bar breakdown and re-locks within 2 bars, and section
changes match the user's marks within one bar (pause and drop within one
beat) with Link on, off and wrong.

**Phase 2: The playbook.** Moves, looks, playbook, bag rotation, ramps,
impact, phrase-line changes, using the effects that exist and no roles
yet (everything aims at the current wash, motion and accent targets).
Replace `Director::choose`. Rework Set up → Auto to list looks per
section with on/off and a preview. *Done when:* tests prove a section
never plays the same look twice running, every look is used before a
repeat, build-ups only ever get the build-up family, and the old Auto
tests still pass or are replaced one for one.

**Phase 3: Stage roles.** Roles, Path order, pairs, fallbacks, layout
suggestions, the role editor in Set up → Stage, stage profiles carrying
roles and signature looks, the two new example stages. *Done when:* the
same playbook renders sensibly on the example stage, the face, the dragon
and a stage with no roles, checked by tests on which fixtures light.

*Phase 3 status:* built: roles per group with fallbacks (`lumen-core/roles.rs`),
path order and "runs backwards" on fixtures, layout suggestions the user must
accept, the role editor, roles saved with the stage profile, the face and
dragon examples, and tests that every look lights something on every stage
with and without roles. Not built yet: signature looks per stage. Note the
lights test cannot fail on a bare stage because the wash covers the rig; the
Director tests (`motion_leads_on_the_role_it_was_written_for` and friends)
are what check that roles change where effects go.

**Phase 4: Effects.** The new effects in D, then fill the playbook to
about ten looks per section. *Done when:* each section has at least ten
enabled looks with flash-safe off, the Drop list has both strobe looks
and bass-driven looks, at least six Drop looks remain with flash-safe
on, and each look has a rendered-frame test.

**Phase 5: Solid.** Soak test, fail-soft behaviour, Show mode, the
before-the-show check. Tidy the stale docs (README links to the old
`lumen` repo, `INSTALL.txt` heading, `ROADMAP.md` ticks,
`docs/design/ui-redesign.md` status). *Done when:* the soak test passes
in CI and the user runs one full set without touching the app.

Order matters: 0 before 1, 1 before 2, 2 before 3 and 4. Phase 5's soak
test can start any time after Phase 0.

## Left to the builder

- The beat-tracking method, as long as it meets Phase 1's numbers and
  the tick budget. No machine-learning models and no new heavy
  dependencies; check any new crate before adding it.
- The exact data shapes for moves, looks, roles and paths.
- The exact contents of the fifty looks. Keep to the families; the user
  will judge them on the rig and switch off what he dislikes.
- The layout heuristics for suggesting roles.
- File and module names.

## Calls made in this document (not yet confirmed by the user)

- Looks are compiled in; the show stores only the user's changes.
- Rotation is a seeded shuffled bag, not a fixed order.
- The audio wins when Link and the audio disagree.
- Flash-safe allows at most 3 flashes a second, and at least six Drop
  looks must still work with it on.
- Loading a stage profile keeps the playbook choices.
- This document lives in `docs/design/` beside `ui-redesign.md`.

## Answers from the user (27 Nov 2026)

1. LINK is on for every deck he plays. (Whether he rides the pitch by
   hand was not answered; the clock rules above cope either way.)
2. Real strobes are wanted in drops, together with effects that work
   with the bass.
3. There is no fixed rig. The stage changes, and changing it must be
   easy.
