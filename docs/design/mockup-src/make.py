#!/usr/bin/env python3
"""Kickline decluttering mockups: Option A (Play / Set up) and Option B (inspector)."""

RAINBOW = "linear-gradient(90deg,#f33,#f90,#ff3,#3d3,#3cf,#36f,#c3f)"
PLASMA = "radial-gradient(circle at 30% 40%,#ff5fa0,transparent 45%),radial-gradient(circle at 70% 60%,#5fd0ff,transparent 45%),#3a2a60"
FIRE = ["#ff2000", "#ff7a00", "#ffc400", "#ff0040"]
PALS = [FIRE, ["#00b4ff", "#0040ff", "#80f0ff", "#fff"], ["#80ff00", "#00ff60", "#ffee00", "#00ffd0"], ["#ff3060", "#ff8a00", "#8020ff", "#ff00a0"], ["#f00", "#03f", "#fff", "#f00"], ["#60f", "#c0f", "#20a", "#f0f"], ["#0fb", "#09f", "#02a", "#0ff"]]


def strip_thumb(kind):
    # four mini strips with an effect drawn
    rows = []
    for k in range(4):
        if kind == "meteor":
            off = 10 + k * 0
            g = f"linear-gradient(90deg,transparent {off}%,#ff5a20 {off+18}%,#fff {off+22}%,transparent {off+23}%)"
        elif kind == "wave":
            off = 55 - k * 12
            g = f"linear-gradient(90deg,transparent {off-20}%,#ffc400 {off}%,transparent {off+1}%)"
        elif kind == "burst":
            g = "radial-gradient(ellipse at 50% 50%,#fff 0 6%,rgba(255,255,255,.5) 7% 20%,transparent 21%)"
        elif kind == "fill":
            g = "linear-gradient(90deg,#ffc400 0 62%,transparent 62%)"
        elif kind == "sparkle":
            g = f"radial-gradient(circle at {20 + k * 7}% 50%,#fff 0 2px,transparent 3px),radial-gradient(circle at 63% 50%,#fff 0 2px,transparent 3px),radial-gradient(circle at {40 + k * 13}% 50%,#fff 0 2px,transparent 3px)"
        else:  # meter
            g = "linear-gradient(90deg,transparent 22%,#ff2000 22% 78%,transparent 78%)"
        rows.append(f'<i style="background:{g}"></i>')
    return '<div class="strip" style="padding-top:6px">' + "".join(rows) + "</div>"


def cell(name, bg=None, on=False, strip=None, h=62):
    if name is None:
        return f'<div class="cell empty" style="height:{h}px"><div class="n">+</div></div>'
    inner = strip_thumb(strip) if strip else ""
    style = f"height:{h}px;" + (f"background:{bg};" if bg else "")
    return f'<div class="cell{" on" if on else ""}" style="{style}">{inner}<div class="n">{name}</div></div>'


LAYERS = [
    ("Strip effects", "#f0a030", [("Meteor", None, False, "meteor"), ("Kick burst", None, True, "burst"), ("Hi-hat sparkle", None, False, "sparkle"), ("Bass meter", None, False, "meter"), ("Fill to the bar", None, False, "fill"), ("Wave", None, False, "wave")], "Add", 100, "Nothing"),
    ("Kick", "#e8607a", [("Kick white", "#101010", True, None), ("Kick red", "#2a0505", False, None), ("Hat blue", "#05102a", False, None), None, None, None], "Add", 60, "Kick"),
    ("Snare chases", "#9a7cf0", [("White chase", "linear-gradient(90deg,#000 60%,#fff 70%,#000 80%)", True, None), ("Strobe", "#eee", False, None), ("Palette chase", "linear-gradient(90deg,#000 30%,#ff7a00 45%,#000 60%)", False, None), None, None, None], "Add", 80, "High mid"),
    ("Bass background", "#6fc49a", [("Rainbow", RAINBOW, True, None), ("Plasma", PLASMA, False, None), ("Red", "#e00", False, None), ("Blue", "#04f", False, None), ("Bands", "linear-gradient(0deg,#3d3 0 30%,transparent 30%),#111", False, None), ("Palette wash", "linear-gradient(90deg,#ff2000,#ff7a00,#ffc400,#ff0040)", False, None)], "Normal", 100, "Nothing"),
]


def palettes_row():
    sw = "".join(f'<div class="sw{" on" if i == 0 else ""}">' + "".join(f'<i style="background:{c}"></i>' for c in p) + "</div>" for i, p in enumerate(PALS))
    return f'<div class="row" style="gap:8px"><span class="h" style="margin-right:4px">Palette</span>{sw}<span class="btn">Next ▸</span><span class="weak" style="font-size:13px;margin-left:6px">changes on Bar · every 4</span></div>'


def preview(w, h, dots=True):
    s = f'<div style="width:{w}px;height:{h}px;border-radius:8px;background:{RAINBOW};position:relative;overflow:hidden;opacity:.95">'
    if dots:
        s += '<div style="position:absolute;inset:0;background:rgba(0,0,0,.35)"></div>'
        for y in (int(h * .62), int(h * .8)):
            s += f'<div style="position:absolute;left:8%;right:8%;top:{y}px;height:6px;border-radius:3px;background:linear-gradient(90deg,#222 20%,#ff5a20 34%,#fff 37%,#222 38%)"></div>'
        for x in (12, 34, 56, 78):
            s += f'<div style="position:absolute;left:{x}%;top:14%;width:16px;height:16px;border-radius:4px;background:#ffd27a;box-shadow:0 0 14px #ffb040"></div>'
    return s + "</div>"


def page(title, body):
    return f'<!doctype html><html><head><meta charset="utf-8"><link rel="stylesheet" href="base.css"><title>{title}</title></head><body>{body}</body></html>'


def top(mode_on, extra=""):
    seg = f'<div class="seg"><span class="{"on" if mode_on == "play" else ""}">Play</span><span class="{"on" if mode_on == "setup" else ""}">Set up</span></div>'
    return f'''<div class="top">
<span class="chip mono">124.0 BPM</span><span class="row" style="gap:5px"><span class="weak">Link</span><span class="dot" style="background:var(--acc)"></span><span class="dot"></span><span class="dot"></span><span class="dot"></span></span>
<span class="row" style="gap:10px;width:210px"><span class="weak">Master</span><div class="fader" style="flex:1"><i style="width:88%"></i><b style="left:88%"></b></div></span>
<span class="btn">Blackout</span>
<span class="spacer"></span>{extra}
<span class="badge live">● OUTPUT LIVE · 6 universes</span>
{seg}
<span class="btn">Club Sanctuary ✓</span></div>'''


# ---------------------------------------------------------------- Option A: Play
def a_play():
    rows = ""
    for name, col, clips, blend, op, react in LAYERS:
        cells = "".join(cell(*c, h=74) if c else cell(None, h=74) for c in clips)
        rows += f'''<div class="row" style="gap:10px;align-items:stretch">
  <div style="width:150px;padding:8px 10px;border-left:4px solid {col};background:var(--panel);border-radius:8px;display:flex;flex-direction:column;justify-content:space-between">
    <div class="row" style="justify-content:space-between"><span class="h">{name}</span><span class="weak">⋯</span></div>
    <div class="fader" style="margin:6px 4px 2px"><i style="width:{op}%;background:{col}"></i><b style="left:{op}%;width:14px;height:14px;top:-4px;margin-left:-7px"></b></div>
    <div class="row" style="gap:6px;font-size:12px"><span class="chip" style="padding:2px 7px">M</span><span class="weak">{('♪ ' + react) if react != 'Nothing' else ''}</span></div>
  </div>
  <div style="flex:1;display:grid;grid-template-columns:repeat(6,1fr);gap:8px">{cells}</div>
</div>'''
    cols = "".join(f'<span class="btn" style="text-align:center">▶ {i}</span>' for i in range(1, 7))
    body = top("play") + f'''
<div style="flex:1;display:flex;gap:14px;padding:14px">
  <div style="flex:1;display:flex;flex-direction:column;gap:12px">
    {palettes_row()}
    <div class="row" style="gap:10px"><div style="width:150px"></div><div style="flex:1;display:grid;grid-template-columns:repeat(6,1fr);gap:8px">{cols}</div></div>
    {rows}
  </div>
  <div style="width:300px;display:flex;flex-direction:column;gap:12px">
    <div class="sec">What the lights get</div>
    {preview(300, 170)}
    <div class="card" style="padding:12px;display:flex;flex-direction:column;gap:10px">
      <div class="sec">Music</div>
      <div class="row" style="justify-content:space-between"><span>rekordbox</span><span class="weak mono">48 kHz</span></div>
      <div class="row" style="gap:14px"><span class="row" style="gap:6px"><span class="dot" style="background:#e8607a;box-shadow:0 0 10px #e8607a"></span>Kick</span><span class="row" style="gap:6px"><span class="dot" style="background:#f0c040"></span>Snare</span><span class="row" style="gap:6px"><span class="dot" style="background:#5fb4ff"></span>Hats</span></div>
    </div>
    <div class="card" style="padding:12px;display:flex;flex-direction:column;gap:8px">
      <div class="sec">Lights</div>
      <div class="row" style="justify-content:space-between"><span class="row" style="gap:7px"><span class="dot" style="background:var(--ok)"></span>H807SA stage left</span><span class="weak mono">U0–3</span></div>
      <div class="row" style="justify-content:space-between"><span class="row" style="gap:7px"><span class="dot" style="background:var(--ok)"></span>Pars</span><span class="weak mono">U10</span></div>
    </div>
    <div class="spacer"></div>
    <div class="weak" style="font-size:12px;line-height:1.4">Everything for playing, nothing for setting up. Layer settings are behind ⋯. Right-click a clip to edit it.</div>
  </div>
</div>'''
    return page("A Play", body)


# ---------------------------------------------------------------- Option A: Set up
def a_setup():
    nav = "".join(f'<div style="padding:9px 14px;border-radius:7px;{"background:var(--raised);color:var(--text);font-weight:700" if n == "Output" else "color:var(--weak)"}">{n}</div>' for n in ["Output", "Fixture library", "Music", "DMX values", "Diagnostics"])
    lvs = ""
    for nm, u, on, sel in [("H807SA stage left", "U0–3 · 2.0.0.18", True, False), ("Pars", "U10 · 2.0.0.20", True, True)]:
        lvs += f'<div style="padding:9px 10px;border-radius:7px;{"background:var(--raised);outline:1.5px solid var(--acc)" if sel else ""}"><div class="row" style="gap:8px"><span class="dot" style="background:var(--ok)"></span><span class="h">{nm}</span></div><div class="weak mono" style="margin:3px 0 0 17px">{u}</div></div>'
    fx = "".join(f'<div class="row" style="justify-content:space-between;padding:6px 10px;border-radius:6px;{"background:var(--raised)" if n == "Par 3" else ""}"><span>{n}</span><span class="weak mono">{a}</span></div>' for n, a in [("Strip 1–8", "U0–3"), ("Par 1", "U10 1"), ("Par 2", "U10 5"), ("Par 3", "U10 9"), ("Par 4", "U10 13")])
    canvas = f'<div style="position:relative;height:440px;border-radius:10px;background:{RAINBOW};overflow:hidden"><div style="position:absolute;inset:0;background:rgba(0,0,0,.25)"></div>'
    for i in range(8):
        canvas += f'<div style="position:absolute;left:6%;right:6%;top:{200 + i*26}px;height:10px;border:1px solid rgba(255,255,255,.8);border-radius:3px"></div>'
    for k, x in enumerate((10, 32, 54, 76)):
        sel = k == 2
        canvas += f'<div style="position:absolute;left:{x}%;top:40px;width:90px;height:90px;border:{"2.5px solid var(--acc)" if sel else "1px solid rgba(255,255,255,.8)"};border-radius:4px"><div style="position:absolute;left:6px;top:4px;font-size:13px">Par {k+1}</div><div style="position:absolute;left:38px;top:38px;width:14px;height:14px;border-radius:50%;background:var(--acc)"></div></div>'
    canvas += "</div>"
    ch = "".join(f'<div class="row" style="gap:8px"><span class="mono weak" style="width:30px">{a}</span><span style="width:52px">{n}</span><div style="flex:1;height:8px;border-radius:3px;background:var(--raised)"><div style="width:{v/2.55:.0f}%;height:100%;border-radius:3px;background:{c}"></div></div><span class="mono" style="width:28px;text-align:right">{v}</span></div>' for a, n, v, c in [(9, "Red", 0, "#e33"), (10, "Green", 116, "#3c5"), (11, "Blue", 93, "#47f"), (12, "White", 108, "#eee")])
    body = top("setup") + f'''
<div style="flex:1;display:flex;min-height:0">
  <div style="width:190px;padding:14px 10px;border-right:1px solid var(--line);display:flex;flex-direction:column;gap:4px">{nav}
    <div class="spacer"></div><div class="weak" style="font-size:12px;line-height:1.4;padding:6px">Set-up pages replace the floating windows. Nothing covers the clips while you play.</div></div>
  <div style="width:250px;padding:14px;border-right:1px solid var(--line);display:flex;flex-direction:column;gap:8px">
    <div class="row" style="justify-content:space-between"><span class="sec">Outputs</span><span class="btn" style="padding:3px 9px;font-size:13px">Find…</span></div>
    {lvs}<div class="weak" style="padding:4px 10px">+ Add output</div>
    <div class="row" style="justify-content:space-between;margin-top:10px"><span class="sec">Fixtures</span><span class="btn" style="padding:3px 9px;font-size:13px">+ Add</span></div>
    {fx}
  </div>
  <div style="flex:1;padding:14px;display:flex;flex-direction:column;gap:10px">
    <div class="row" style="justify-content:space-between"><span class="weak" style="font-size:13px">Drag to move · corner to resize · ring to rotate</span><span class="row" style="gap:8px"><span class="chip" style="font-size:13px">Show what the lights get</span><span class="chip" style="font-size:13px">Snap</span></span></div>
    {canvas}
  </div>
  <div style="width:290px;padding:14px;border-left:1px solid var(--line);display:flex;flex-direction:column;gap:12px">
    <div><div class="h" style="font-size:18px">Par 3</div><div class="weak">RGBW par · Pars · U10 9–12</div></div>
    <div class="row" style="gap:8px"><span class="btn acc">Test this light</span><span class="btn">Edit type</span></div>
    <div class="card" style="padding:10px;display:flex;flex-direction:column;gap:7px"><div class="sec">Live channels</div>{ch}</div>
    <div class="card" style="padding:10px"><div class="row" style="justify-content:space-between"><span class="h">Address</span><span class="weak">U10 · 9 ▾</span></div></div>
    <div class="card" style="padding:10px"><div class="row" style="justify-content:space-between"><span class="h">Position &amp; size</span><span class="weak">▸</span></div></div>
    <div class="card" style="padding:10px"><div class="row" style="justify-content:space-between"><span class="h">Wiring &amp; colour</span><span class="weak">▸</span></div><div class="weak" style="font-size:13px;margin-top:4px">White emitter only</div></div>
  </div>
</div>'''
    return page("A Setup", body)


# ---------------------------------------------------------------- Option B
def b_one():
    layers = ""
    for i, (name, col, clips, blend, op, react) in enumerate(LAYERS):
        layers += f'<div style="padding:8px 10px;border-radius:7px;border-left:4px solid {col};{"background:var(--raised)" if i == 1 else "background:var(--panel)"}"><div class="row" style="justify-content:space-between"><span class="h">{name}</span><span class="mono weak" style="font-size:12px">{op}%</span></div></div>'
    grid = ""
    for name, col, clips, *_ in LAYERS:
        grid += '<div style="display:grid;grid-template-columns:repeat(6,1fr);gap:8px">' + "".join(cell(*c, h=66) if c else cell(None, h=66) for c in clips) + "</div>"
    tabs = "".join(f'<span style="padding:7px 14px;border-radius:7px 7px 0 0;{"background:var(--panel);color:var(--text);font-weight:700" if t == "Output" else "color:var(--weak)"}">{t}</span>' for t in ["Output", "Fixture library", "Music", "DMX values", "Diagnostics"])
    canvas = f'<div style="position:relative;height:150px;border-radius:8px;background:{RAINBOW};overflow:hidden"><div style="position:absolute;inset:0;background:rgba(0,0,0,.25)"></div>'
    for i in range(3):
        canvas += f'<div style="position:absolute;left:6%;right:40%;top:{78 + i*20}px;height:8px;border:1px solid rgba(255,255,255,.8);border-radius:3px"></div>'
    for x in (8, 22, 36, 50):
        canvas += f'<div style="position:absolute;left:{x}%;top:16px;width:46px;height:46px;border:1px solid rgba(255,255,255,.8);border-radius:4px"></div>'
    canvas += "</div>"
    insp = f'''<div class="h" style="font-size:18px">Kick</div><div class="weak">Layer · 3 clips</div>
<div class="card" style="padding:10px;display:flex;flex-direction:column;gap:10px">
  <div class="row" style="justify-content:space-between"><span class="lbl">Opacity</span><span class="mono">60%</span></div><div class="fader"><i style="width:60%;background:#e8607a"></i><b style="left:60%"></b></div>
  <div class="row" style="justify-content:space-between"><span class="lbl">Blend</span><span class="field">Add ▾</span></div>
  <div class="row" style="justify-content:space-between"><span class="lbl">Reacts to</span><span class="field">Kick ▾</span></div>
  <div class="row" style="justify-content:space-between"><span class="lbl">Next clip on</span><span class="field">Off ▾</span></div>
</div>
<div class="card" style="padding:10px"><div class="row" style="justify-content:space-between"><span class="h">Effects</span><span class="weak">+ Add</span></div></div>
<div class="card" style="padding:10px"><div class="sec" style="margin-bottom:6px">Clip: Kick white</div><div class="row" style="justify-content:space-between"><span class="lbl">Colour</span><span class="row" style="gap:5px">{''.join(f'<i style="width:18px;height:18px;border-radius:4px;display:inline-block;background:{c}"></i>' for c in FIRE)}</span></div></div>'''
    body = f'''<div class="top">
<span class="chip mono">124.0 BPM</span><span class="row" style="gap:5px"><span class="weak">Link</span><span class="dot" style="background:var(--acc)"></span><span class="dot"></span><span class="dot"></span><span class="dot"></span></span>
<span class="row" style="gap:10px;width:210px"><span class="weak">Master</span><div class="fader" style="flex:1"><i style="width:88%"></i><b style="left:88%"></b></div></span>
<span class="btn">Blackout</span><span class="spacer"></span><span class="badge live">● OUTPUT LIVE · 6 universes</span><span class="btn">Club Sanctuary ✓</span></div>
<div style="flex:1;display:flex;flex-direction:column;min-height:0">
  <div style="flex:1;display:flex;gap:12px;padding:12px;min-height:0">
    <div style="width:170px;display:flex;flex-direction:column;gap:8px"><div class="sec">Layers</div>{layers}<div class="weak" style="padding:4px 10px">+ Layer</div></div>
    <div style="flex:1;display:flex;flex-direction:column;gap:8px">{palettes_row()}{grid}</div>
    <div style="width:270px;padding-left:12px;border-left:1px solid var(--line);display:flex;flex-direction:column;gap:10px">{insp}</div>
  </div>
  <div style="flex:none;height:228px;border-top:1px solid var(--line);background:var(--bg)">
    <div class="row" style="gap:2px;padding:6px 12px 0">{tabs}<span class="spacer"></span><span class="weak" style="font-size:13px">drag to resize · ⌄ hide</span></div>
    <div style="background:var(--panel);height:190px;padding:10px 12px;display:flex;gap:14px">
      <div style="width:220px;display:flex;flex-direction:column;gap:6px"><div class="row" style="gap:8px"><span class="dot" style="background:var(--ok)"></span>H807SA stage left <span class="weak mono">U0–3</span></div><div class="row" style="gap:8px"><span class="dot" style="background:var(--ok)"></span>Pars <span class="weak mono">U10</span></div><div class="weak" style="margin-top:6px">+ Add output · Find…</div></div>
      <div style="flex:1">{canvas}</div>
    </div>
  </div>
</div>'''
    return page("B", body)


for name, fn in [("a_play", a_play), ("a_setup", a_setup), ("b_inspector", b_one)]:
    open(f"/tmp/mock/{name}.html", "w").write(fn())
print("ok")
