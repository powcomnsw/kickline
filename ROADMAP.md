# Roadmap

Goal: match the parts of Resolume (Avenue/Arena) that matter for lighting, in the open.

## Architecture

```
 inputs                engine (lumen-core)              mapping              outputs
 ───────               ───────────────────              ───────              ───────
 UI / MIDI / OSC ──►  Composition                       Patch                Art-Net
 Audio / BPM    ──►    layers[] ─ clips[] ─ generator    fixtures[] ─ points  sACN
                       effects[] ─ blend ─ opacity       profile → channels   (later) DDP,
                       clock (beats) ─ master            canvas sample → DMX   serial DMX
                          │                                  ▲
                          └────────── Frame (RGBA f32) ──────┘
```

Key decisions:
- **Everything is beat-time.** Generators take `beat`, not seconds, so BPM sync is free.
- **CPU canvas first.** The canvas is small (pixel-mapping, not 4K video), so a CPU
  compositor is fast enough and trivially testable. A wgpu path is a later
  optimisation, only needed for video clips and heavy shaders.
- **Engine is headless.** `lumen-core` / `lumen-fixtures` / `lumen-output` never touch the GUI.
- **Project = JSON.** Serde on composition and patch.

## Milestones

### M1 — Usable for small shows
- [x] Clip editing in UI (right-click: source, parameters, rename) — drag to reorder still to do
- [x] Add / remove layers — reorder layers, add columns still to do
- [x] Advanced Output: lumiverses, fixtures placed by dragging/resizing/rotating, Fixture Editor, DMX Monitor
- [ ] Tap tempo, beat-phase nudge, beat-quantised triggering
- [ ] Per-clip speed multiplier and transport (loop / one-shot)
- [ ] Blackout and per-layer solo
- [ ] File dialogs, autosave, undo/redo

### M2 — Control surfaces
- [ ] MIDI input with learn mode (faders, pads, column launch)
- [ ] OSC in/out (TouchOSC, Lemur, Resolume-compatible address space where practical)
- [ ] Crossfader / A-B decks
- [ ] Clip and layer transition times (fade in/out)

### M3 — Real fixture support
- [x] Fixture library: 1,781 built-in QLC+ fixtures, `.qxf` import (covers Open Fixture Library and GDTF via OFL)
- [x] Multi-mode profiles, 16-bit fine channels, per-pixel heads, CMY/UV
- [ ] Drive pan/tilt, colour wheels, gobos and strobe from Lumen (currently held at safe values)
- [ ] Moving-head positioning from canvas (or XY pad layer)
- [ ] Gamma / white-point / per-fixture colour calibration
- [ ] Bilinear sampling and area-averaged sampling for fixtures that cover many pixels
- [x] Per-lumiverse protocol and target IP
- [x] Choose network interface per lumiverse, automatic reconnect
- [x] Array tool (rows/columns, zig-zag, universe per strip), RGBW white mode

### M4 — Media and audio
- [ ] Video / image / GIF clips (ffmpeg or GStreamer), loop and scrub
- [x] Audio input (incl. Windows loopback), 5 bands + level, kick/snare/hat detection, per-layer routing
- [ ] Auto-BPM from detected kicks, beat-quantised clip changes
- [x] Ableton Link tempo/beat sync (rekordbox Performance mode)
- [x] rekordbox direct capture on macOS 14.2+ (process tap) — needs first test on a real Mac
- [ ] Pro DJ Link (CDJs / rekordbox Export mode)
- [x] Colour palettes with automatic changes and crossfade
- [ ] Cue list / scenes (palette + clips + master saved and recalled)
- [ ] Parameter modulation: LFOs, envelopes, audio and MIDI as sources
- [ ] Shader generators/effects (WGSL via wgpu) with hot reload
- [ ] NDI / Syphon / Spout input

### M5 — Performance and polish
- [ ] wgpu compositor for large canvases
- [x] Output thread decoupled from UI (fixed-rate DMX, instant send on hits)
- [x] Art-Net node discovery (ArtPoll): Find nodes with one-click setup, live node status per lumiverse
- [ ] sACN priority and merge, DDP, serial DMX / USB adapters
- [ ] Plugin API for generators and effects
- [x] Diagnostics probe, crash capture and exportable report
- [x] macOS .app/.dmg build script + CI workflow (needs first run on a real Mac)
- [ ] Developer ID signing + notarisation (needs an Apple Developer account)
- [ ] Windows / Linux installers

## Known limitations of v0.6
- Imported moving heads light up but don't move yet: pan/tilt are held centred.
- Drum detection is tuned for four-on-the-floor dance music; use the Tuning sliders for other styles.
- Loopback capture is built in on Windows only; macOS needs BlackHole/Loopback.
- Resolume's FFGL/ISF plugins, video codecs (DXV) and NDI are out of scope for v0.1.
