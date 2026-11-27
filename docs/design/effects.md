# More effects

Status: **plan only** (27 Nov 2026). Nothing here is built yet.

Written for the model (or person) who will build it. It sits beside
`auto-vj.md`; read that first, because Auto has to be able to use
everything built here.

## Intent

The main complaint about Kickline, after Auto, is that there are not
enough effects. The user wants far more to choose from, for himself in
the effect browser and for Auto's playbook, without every new look being
a hand-written piece of code.

## Today

- **26 effects.** 16 canvas generators in the `Generator` enum
  (`crates/lumen-core/src/generator.rs`) and 10 strip presets
  (`StripClip::presets` in `strip.rs`). Each is one arm of a `match`.
- **6 modifiers** in the `Effect` enum (`effect.rs`): brightness, hue
  shift, invert, pulse, audio brightness, audio hue. All are colour-only
  and stateless: `apply(&self, frame, beat, audio)`.
- **The browser shows one tile per kind.** `scenes::catalog` lists one
  default setting of each generator, so direction, speed, width, mirror
  and ripple are hidden inside the clip's settings.
- **Names carry the meaning.** `scenes::name` works a name out from the
  generator, and `scenes::tags_for` looks Auto's tags up by that name.
  Two settings of the same generator cannot have different names or tags.
- **Strip effects are a separate path.** They are worked out per pixel at
  output time (`Composition::strip_stage`), not drawn on the canvas, so
  canvas modifiers cannot bend them.
- **No media.** No images, GIFs, video or shaders. `image` and `wgpu` are
  already in `Cargo.lock` through eframe, but nothing of Kickline's uses
  them for content.

## Goals

- G1. At least **150 named effects** in the browser, up from 26.
- G2. At least **12 modifiers** that work on any canvas effect, so the
  combinations run into the thousands.
- G3. The user can **save his own effect** (an effect, its settings and
  its modifiers) and use it in any show.
- G4. The user can **drop an image or a GIF** on the window and play it
  on the rig.
- G5. A decision, backed by a working spike, on **shader effects**.
- G6. Auto can use everything: every built-in effect and modifier carries
  the tags the playbook needs.

## Non-goals

Video files and codecs, NDI / Syphon / Spout input, Resolume FFGL
plugins, a plugin API, and a node-graph effect editor.

## Anti-goals

- A long list of tiles that all look the same. Every named effect must
  be visibly different from its neighbours on a real rig.
- Effects that only look right on a wide canvas and turn to mush on a
  30-pixel strip or a 16×32 panel.
- A modifier offered on a layer where it does nothing.
- A show that fails to open because a media file is missing.
- The engine tick going over budget because of one heavy effect.
- Anything bundled whose licence is not clear.

## Constraints

- Same as `auto-vj.md`: crate layout, `lumen-core` free of I/O and GUI,
  JSON shows with `#[serde(default)]` on new fields, v0.12 shows must
  open, British spelling and plain words in the UI.
- One engine tick stays under 5 ms on the example stage in a release
  build, with any effect and three modifiers on every layer.
- `Generator` and `Effect` are serialised by variant name. Add variants;
  never rename or remove one.
- Effects take palette slots, not fixed colours, unless colour is the
  point of the effect (rainbow, fire).
- Every effect and modifier that can flash declares its highest flash
  rate, so flash-safe can filter it.
- Check any new crate before adding it. Prefer what is already in the
  lock file.

## Every effect is done when

This is the bar for each single effect or modifier, in every track:

1. It has a name and a one-line description in plain words.
2. It has Auto's tags: role, energy range, sections, flash rate.
3. Its tile in the browser animates.
4. A test renders it at several beats and checks it is not blank, not
   stuck, and inside the time budget.
5. A test saves and loads it.
6. It respects a layer aimed at a group (it draws inside the group's
   area, as `Frame::area` already allows).
7. It reads well on three sizes: a single strip, a tall panel, and the
   whole canvas. A test renders all three.

Build one harness that runs checks 4 to 7 over the whole catalogue, so a
new effect is covered by adding it to the list.

## Track 1: Modifiers

Add to the `Effect` enum. Each modifier has one main amount, and that
amount can follow an audio band (kick, bass, and so on), generalising
what `AudioBrightness` and `AudioHue` do today.

Wanted, at least:

| Modifier | What it does |
|---|---|
| Mirror | left/right, top/bottom, or both |
| Kaleidoscope | repeats a wedge around the middle |
| Tile | repeats the picture in a grid |
| Rotate | a fixed angle, or turning in time with the beat |
| Scroll | slides the picture, wrapping round |
| Zoom | a fixed size, or pumping with a band |
| Trails | each frame fades into the next |
| Strobe gate | chops the layer on and off on the beat |
| Pixelate | bigger blocks |
| Posterise | fewer brightness steps |
| Colourise | repaints brightness with palette colours |
| Mask | shows the layer only through a shape: circle, bars, checker |
| Slice shuffle | cuts the picture into bands that swap on the beat |
| Wave | bends the picture with a moving wave |

Things to settle while building:

- **Trails needs memory.** Modifiers are stateless today. Keep the state
  beside the layer and out of the saved show, the same way
  `Layer::trigger_beat` is skipped.
- **Order matters.** Modifiers run top to bottom in the layer's list;
  the list must be reorderable.
- **Strip effects.** Space-bending modifiers cannot apply to strip
  clips. Give strip clips the ones that make sense along a strip (strobe
  gate, colourise, trails) and hide the rest for strip layers.
- **Auto.** A look in the playbook may add modifiers to a move. This is
  the cheap way to make drop 2 differ from drop 1, so tag each modifier
  with the sections it suits.

## Track 2: Named presets and the user's own effects

- **The bank becomes a table.** Replace "work the name out from the
  generator, then look the tags up by name" with one list of entries:
  name, description, category, generator with its settings, optional
  modifiers, tags. `scenes::catalog`, `scenes::bank` and `tags_for`
  become views of that list. Existing clips keep the names they have.
- **Presets.** Ship the good settings of each generator as their own
  entries: directions, speeds (4, 2, 1 and ½ beats), widths, mirrored,
  rippling across strips, driven by each drum. Only settings that look
  different on a rig. The browser groups them under their kind, so the
  first screen stays short.
- **Browser.** With 150 or more tiles it needs search by name and
  filters by category, by "reacts to" (kick, snare, hats, bass, beat)
  and by "safe for flash-safe".
- **My effects.** "Save as my effect" on any clip stores the generator,
  its settings and the layer's modifiers under a name the user types.
  They are kept in `~/Documents/Kickline/Effects` as JSON, one file per
  effect, so they follow the user across shows and can be copied to
  another Mac. They appear in the browser under their own category, and
  the user can tag them for Auto.

## Track 3: New hand-built effects

Around 30 new ones. The list is a starting point; the builder may swap
ideas of equal worth, but the count and the spread across families
stand.

**Canvas:** fire · ripples from the centre on each kick · tunnel of rings
· spiral · lightning on a drum · starfield · rain · equaliser bars ·
checker swap · scan lines · bouncing balls · particle burst on the kick ·
drifting noise clouds · rotating palette gradient · aurora · pulsing
shape outlines (circle, triangle, square) · diagonal stripes · confetti ·
heartbeat · orbiting comet.

**Along strips:** several meteors in a train · theatre chase · scanner
that bounces end to end · pixels stacking up from one end · fire along
the strip · bass fill from the middle · two dots that collide and flash ·
scrolling palette gradient · random flicker.

**Bass-driven** (the user asked for these by name): a throb whose
brightness follows the bass · a bloom that grows from the centre with the
bass level · a bass meter with a falling peak marker.

Notes:

- `auto-vj.md` section D already lists effects the playbook needs
  (one-shots, breathe, impact, eye glow, a meteor along a whole Path).
  Build those there. Do not build them twice; share the machinery.
- `generator.rs` is one file with one large `match`. With 50 or more
  generators, split it by family. The enum and its saved names stay.
- Random-looking effects use the repeatable `hash` already in the file,
  never a real random source, so tests and replays stay exact.

## Track 4a: Images and GIFs

- **Use.** Drop a `.png`, `.jpg` or `.gif` on the window, or pick one in
  the browser's new "My media" category. It becomes a clip like any
  other: layers, blend, opacity, modifiers, group targets and Auto all
  work.
- **Where files live.** Copied into `~/Documents/Kickline/Media`, the
  same pattern as dropped fixture files. The show stores the file name,
  not the pixels and not an absolute path.
- **Missing file.** The clip plays nothing, the tile says the file is
  missing and offers to find it. The show still opens and saves.
- **Core stays free of I/O.** The app decodes; `lumen-core` is handed
  the frames. That means the render call needs a way to reach decoded
  media. Decode once, off the engine thread, already scaled to the
  canvas. Set a limit on file size and frame count and say so plainly
  when a file is over it.
- **Settings per clip:** fit (stretch, fill, fit), and for GIFs the loop
  length in beats (so it locks to the music) or its own speed.
- **Tinting.** A switch to repaint a picture with the palette, using the
  Colourise modifier, so media follows the show's colours.
- **Bundled media.** If any is shipped, it must be made for Kickline or
  be public-domain, with the source written down.
- **Video is out.** A user with a video can convert it to a GIF.

## Track 4b: Shaders

Shaders would open up large community libraries, but they are the
biggest and riskiest piece, so this track starts with a spike and a
decision.

**The spike answers:**

1. Can a small shader run off-screen on the wgpu device the app already
   has, and its pixels be read back into a `Frame` on the engine thread,
   every tick, inside the 5 ms budget, with the window hidden?
2. How late do the pixels arrive? More than one tick is too late for
   effects that hit on the kick.
3. What happens with no usable GPU, or when the GPU is lost? The answer
   must be "that clip plays nothing and the rest of the show carries
   on".
4. Which shader language? WGSL is native to wgpu. The big free libraries
   are GLSL (ISF format); `naga`, already a dependency, can translate
   some GLSL. Measure how many real ISF shaders load unchanged.

**Then decide,** with the user, one of: build it; build it for WGSL
only; or leave it and put the effort into more hand-built effects. Write
the decision and the numbers into this file.

**If it is built:** shader files live in `~/Documents/Kickline/Shaders`;
a shader gets the beat, the bar position, the five bands, the drum hits
and the four palette colours as inputs; editing the file reloads it
live; a shader that fails to compile shows its error on the tile and
plays nothing. Only bundle shaders whose licence allows it, and keep the
licence beside each one.

## Order

| Step | What | Depends on |
|---|---|---|
| E1 | The bank as a table, the whole-catalogue test harness, named presets, browser search and filters (Track 2 without "My effects") | nothing |
| E2 | Modifiers (Track 1), then "My effects" | E1 |
| E3 | New effects (Track 3) | E1 |
| E4 | Images and GIFs (Track 4a) | E2, for tinting |
| E5 | Shader spike and decision (Track 4b) | nothing; can run any time |

E2 and E3 can be built side by side.

**Working alongside `auto-vj.md`.** Both plans change `scenes.rs`, and
Auto VJ Phase 2 replaces the code that reads the bank. Do E1 first, or
agree the shape of the bank table with whoever builds Phase 2 before
either starts. After that, effects and the clock work do not touch the
same files.

Each step ends with the app releasable and a short list for the user to
try on the rig.

## Left to the builder

- The exact presets, as long as each is visibly different.
- Which 30 new effects, within the families above.
- The data shape of the bank table and of saved user effects.
- How decoded media reaches the render call.
- File and module layout.

## Calls made in this document (not yet confirmed by the user)

- The targets: 150 named effects, 12 modifiers, about 30 new effects.
- Video stays out; GIFs cover moving pictures.
- The user's own effects and media are files in `~/Documents/Kickline`,
  shared by all shows, not stored inside one show.
- Shaders start with a spike and a decision, not a commitment.

## Open question for the user

Who is asking for more effects, and what do they say is missing? "It all
looks the same after an hour" points at modifiers and Auto. "There is no
fire, no lightning, nothing for my logo" points at new effects and
media. The answer decides which step goes first after E1.
