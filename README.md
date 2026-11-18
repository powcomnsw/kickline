<p align="center">
  <img src="docs/brand/banner.png" alt="Kickline: lights and LED pixels that play along with your DJ set" width="100%">
</p>

<p align="center">
  <a href="https://github.com/powcomnsw/lumen/releases/latest"><img alt="Latest release" src="https://img.shields.io/github/v/release/powcomnsw/lumen?style=flat-square&color=ff8a1f&label=download"></a>
  <a href="https://github.com/powcomnsw/lumen/actions/workflows/macos.yml"><img alt="Mac build" src="https://img.shields.io/github/actions/workflow/status/powcomnsw/lumen/macos.yml?style=flat-square&label=mac%20build"></a>
  <img alt="macOS 11+" src="https://img.shields.io/badge/macOS-11%2B-2a2018?style=flat-square">
  <img alt="Licence" src="https://img.shields.io/badge/licence-MIT%20%2F%20Apache--2.0-2a2018?style=flat-square">
  <img alt="Written in Rust" src="https://img.shields.io/badge/rust-egui%20%2B%20wgpu-2a2018?style=flat-square">
</p>

<p align="center">
  <b><a href="https://github.com/powcomnsw/lumen/releases/latest">Download for Mac</a></b>
  &nbsp;·&nbsp; <a href="#quick-start">Quick start</a>
  &nbsp;·&nbsp; <a href="#features">Features</a>
  &nbsp;·&nbsp; <a href="ROADMAP.md">Roadmap</a>
  &nbsp;·&nbsp; <a href="#reporting-problems">Report a problem</a>
</p>

---

Kickline is a free, open-source alternative to Resolume's lighting side, made for
DJs who run their own LEDs. Clips on layers are mixed into a canvas, the canvas is
mapped onto your fixtures and pixel strips, and it all goes out over Art-Net or
sACN. It listens to the music itself and splits it into **bass, kick, snare,
hi-hats and mids**, so each part of the track can drive a different part of the
rig.

> **New name.** Kickline used to be called Lumen. From 0.7.0 the app carries the new
> name and icon, and moves your Lumen projects and settings across by itself. macOS
> asks for its permissions (microphone, Local Network, audio recording) once more.

<p align="center">
  <img src="docs/screenshots/main.png" alt="Kickline main window: palette strip, layers and clip grid" width="100%">
</p>

## Features

<table>
<tr>
<td width="50%" valign="top">
<h3>Hears the music the way you do</h3>
<p>Not an FFT spectrum. A filter bank splits the sound into five bands and picks out <b>kick, snare and hi-hat</b> as separate triggers, about 13 ms after they happen. Route any of them to any layer: opacity follows the bass, the next clip fires on every snare, a white flash on each kick.</p>
</td>
<td width="50%" valign="top">
<h3>Built for rekordbox</h3>
<p>On macOS 14.2+ Kickline captures rekordbox's output <b>directly</b>, so no BlackHole and no extra delay. <b>Ableton Link</b> keeps it locked to rekordbox's tempo and beat, with Beat and Bar triggers fired slightly early to cancel out the network's delay.</p>
</td>
</tr>
<tr>
<td valign="top">
<h3>Pixel mapping that doesn't fight you</h3>
<p>Drag, resize and rotate fixtures straight onto the canvas. <b>Add array</b> lays out eight zig-zag strips, each on its own universe, in one go, and skips universes another node already uses. Snake wiring, reverse, RGBW white modes and automatic universe spill are all built in.</p>
</td>
<td valign="top">
<h3>Colour palettes</h3>
<p>Four colours that clips follow instead of fixed colours. Pick a new palette and the whole show recolours with a crossfade, or let it change every few bars, on the kick or on the snare. Eight palettes to start from, all editable.</p>
</td>
</tr>
<tr>
<td valign="top">
<h3>1,781 fixtures built in</h3>
<p>The full QLC+ fixture library, searchable, with every channel explained. Missing one? Drop a <code>.qxf</code> from the Open Fixture Library onto the window.</p>
</td>
<td valign="top">
<h3>Safe for live shows</h3>
<p>DMX runs on its own high-priority thread at a fixed rate, even when the window is hidden. <b>Find nodes</b> lists every Art-Net node and sets a lumiverse up in one click, and a dot per lumiverse turns red if a node stops answering mid-set. Pin each lumiverse to a network port: pull the adapter out, plug it back in, and output <b>resumes by itself</b>.</p>
</td>
</tr>
</table>

### See it

| | |
|---|---|
| ![Advanced Output](docs/screenshots/advanced-output.png) | ![Array tool](docs/screenshots/array.png) |
| **Advanced Output**: lumiverses and fixtures placed on the canvas, with live pixel colours | **Add array**: eight zig-zag strips, each on its own universe |
| ![Colour palettes](docs/screenshots/palettes.png) | ![Fixture library](docs/screenshots/fixture-library.png) |
| **Palettes**: a clip set to palette colour 2, ready to follow palette changes | **Fixture library**: shows what each channel of a fixture does |
| ![Audio](docs/screenshots/audio.png) | ![Diagnostics](docs/screenshots/diagnostics.png) |
| **Audio**: bands, kick/snare/hat, Ableton Link and timing | **Diagnostics**: one-click checks and a report you can send |

## Quick start

1. **Install.** Download the `.dmg` from [Releases](https://github.com/powcomnsw/lumen/releases/latest),
   drag the app into Applications, then right-click it and choose **Open** the first time
   (it isn't signed by Apple yet; see [first launch](packaging/macos/INSTALL.txt)).
2. **Setup guide.** It opens the first time: it finds your controller, asks what
   lights you have (strips, panels or anything from the library), then lets you
   **test each light** for colour order and wiring direction. About five minutes.
3. **Sound.** Open **Audio** and click **Use rekordbox (direct)**. With no music
   playing, **Demo beat** works too.
4. **Play.** Click clips and palettes. Your show **saves by itself**, and **⌘Z** undoes anything.

<details>
<summary><b>Other ways to get rekordbox audio in</b></summary>

- **macOS 14.2+:** Audio › **Use rekordbox (direct)**. No BlackHole needed.
- **Older macOS:** set rekordbox's master output to [BlackHole](https://existential.audio/blackhole/) (free) or Loopback and pick it as the input.
- **Windows:** pick the device rekordbox plays through, listed as "(loopback)".
- **Any computer:** cable the controller's booth or master out into an audio interface.

</details>

<details>
<summary><b>Adding your fixtures</b></summary>

Fixture Editor › **Library**: search for the brand or model, pick the mode
your fixture is set to, then **Add and place in Advanced Output**. The list
shows what Kickline does with every channel.

Not in the list? Download it from [open-fixture-library.org](https://open-fixture-library.org)
as a **QLC+ fixture** (`.qxf`) and drag the file onto the window. It's copied to
`~/Documents/Kickline/Fixtures`. Fixtures from an installed QLC+ are picked up too.

To change a fixture type later, select a fixture and click **Edit type**.

</details>

<details>
<summary><b>Splitting the music across effects</b></summary>

Each layer can react to a different part of the track:
- **Opacity follows** a band (e.g. a layer only shows when the high mids are busy).
- **Next clip on** Kick, Snare or Hi-hat steps through the layer's clips on each hit.
- **Audio flash** clips flash a colour on a band (white on kick, blue on hats).
- **Audio brightness / Audio hue** effects change any clip with the music.

The demo project shows one of each. The Audio window lists the current routing.

</details>

<details>
<summary><b>Colour palettes</b></summary>

The **Palette** strip above the clips holds four colours. Right-click a clip and
pick a palette swatch instead of a fixed colour, and it follows the palette.
Click another palette to recolour the whole show, or set **Change on** to Bar
(every 4 bars, say) for automatic changes. **Fade** sets the crossfade in beats.

</details>

<details>
<summary><b>Networking for live shows</b></summary>

Set each lumiverse's **Network port** to the adapter your node is on (e.g. a USB
Ethernet adapter). Kickline sends only through that port, and if it disappears it
keeps trying and resumes by itself when it's back. Diagnostics shows the
state of each port and how many times output recovered.

</details>

## Reporting problems

Open **Diagnostics**. It checks everything Kickline depends on and explains each
problem in plain language (e.g. *"Host unreachable: allow Kickline under Local
Network"*). Type what went wrong, click **Export report**, and send the
`.json` file it saves to your Desktop. It holds the checks, the event log, any
crash reports and (if you choose) your project. Nothing is uploaded.

## What's done

| Area | |
|---|---|
| Getting started | Setup guide (controller → lights → test → play), "Test this light" for colour order and wiring, plain-language hover help for every lighting term |
| Saving | Autosave, reopens your last show, New / Open / Save as, undo and redo (⌘Z / ⌘⇧Z) that never stops what's playing |
| Look | Three dark themes (Tungsten, Gel, Stage) for dark venues, Atkinson Hyperlegible font, live clip thumbnails, a colour per layer |
| Composition | Layers × clip grid, column triggers, opacity / blend / bypass / effects per layer, master, BPM clock, right-click clip editing |
| Generators | Solid, Rainbow, Strobe, Chase, Plasma, Audio flash, Bands, Palette wash |
| Strip effects | Run along every strip on its own, at any angle: Meteor (and bounce), Fill, Kick burst, Segments, Sparkle, Meter, Dashes, Alternate. Reverse, from-the-middle and a ripple across strips. Shown live in the preview |
| Effects | Brightness, Hue shift, Invert, Pulse, Audio brightness, Audio hue |
| Colour palettes | Clips use palette colour 1–4 or a fixed colour; 8 palettes to start, editable; change on Bar / Beat / Kick / Snare / Hi-hat every N, with a crossfade |
| Smooth output | ArtSync so every universe changes on the same frame, per-controller refresh cap for pixel controllers that glitch when fed too fast |
| Art-Net nodes | Find nodes (ArtPoll) with one-click setup; live status per lumiverse, wrong-universe warnings, alert when a node drops out |
| Advanced Output | Art-Net / sACN lumiverses with their own universe, IP and network port; drag / resize / rotate fixtures; snake and reverse wiring; universe spill; overlap warnings; array tool; RGBW white modes |
| Fixtures | 1,781 built-in (QLC+), `.qxf` import, pixel-grid editor, colour order presets |
| Audio | Live input or loopback, 5 bands + level, kick / snare / hi-hat detection that works at any volume and through loud, sidechained masters, sensitivity per drum with a live meter, per-layer routing |
| rekordbox | Direct capture on macOS 14.2+, Ableton Link tempo and beat sync |
| Timing | Engine on its own thread at a fixed DMX rate (44 Hz default); hits sent about 1 ms after detection |
| Diagnostics | System, audio, network, Art-Net node discovery, project and performance checks; crash capture; exportable report |

Coming next: cue lists, MIDI controllers, moving heads, and video clips. See the
[roadmap](ROADMAP.md).

## For developers

```sh
cargo run --release -p lumen-app
cargo test --workspace
```

Linux needs ALSA headers (`sudo apt install libasound2-dev`).

**Mac app:** on a Mac with Xcode Command Line Tools and Rust:

```sh
./scripts/build-macos.sh          # universal app + dist/<name>-<version>.dmg
./scripts/build-macos.sh --native # this Mac's architecture only, faster
```

Every push to `main` that changes code builds the `.dmg` on GitHub and publishes
it as a release named after the version in `Cargo.toml`.

```
crates/lumen-core      frames, generators, effects, palettes, composition (no I/O)
crates/lumen-fixtures  Advanced Output model: lumiverses, fixture types, arrays, canvas → DMX
crates/lumen-output    Art-Net and sACN packets, network ports, node discovery
crates/lumen-audio     filter bank, band levels, kick/snare/hat detection, audio input
crates/lumen-app       the app (egui + wgpu)
```

The engine crates don't depend on the GUI, so a headless runner, a web UI or a
plugin host could reuse them.

## Licence

Kickline's own code is MIT OR Apache-2.0. Builds with Ableton Link (the default
`link` feature) include Ableton's GPL-2.0-or-later Link library, so those
binaries are distributed under the GPL. Build with `--no-default-features`
for a Link-free MIT/Apache build.

Fonts: Atkinson Hyperlegible and IBM Plex Mono, SIL Open Font License 1.1
(`crates/lumen-app/assets/fonts`). Built-in fixture definitions come from the
[QLC+](https://github.com/mcallegari/qlcplus) project (Apache-2.0); see
`crates/lumen-fixtures/assets/QLCPLUS-LICENSE.txt`.
