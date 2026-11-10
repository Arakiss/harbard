#!/usr/bin/env python3
"""Render the living Dock icon, the tray variants and the macOS 26 layered icon.

Every file this writes is OUTPUT of assets/brand/mark.svg, like the rest of
the brand set. Called by scripts/render-brand.sh; runs on its own too:

    python3 scripts/render-dock.py [--html PATH]

Writes:

- apps/desktop/src-tauri/icons/dock/<state>-<size>.png: one Dock tile per
  lasting state, and <event>-<frame>-<size>.png for each one-shot animation,
  at 512 and 1024. The tile sits on Apple's macOS grid (an 824 px squircle in
  a 1024 canvas), the size the layered bundle icon is drawn at.
- apps/desktop/src-tauri/icons/tray-<variant>.png: menu-bar template images.
  Template images are monochrome, so the state is a shape, not a colour.
- assets/brand/harbard.icon/Assets/*.svg: the layers of the Icon Composer
  document whose icon.json is written by hand.
- apps/desktop/src-tauri/icons/Assets.car, compiled from harbard.icon by
  actool, and Assets.car.sources, the fingerprint of the sources it was
  compiled from. Without actool the existing car is kept only while that
  fingerprint still matches; otherwise the script fails, and build.rs
  refuses to build with a stale car.
- --html PATH: one page with every state and frame at Dock sizes on light and
  dark Docks, plus the layered icon's appearances rendered by ictool.

Needs rsvg-convert. ictool and actool come from /Applications/Xcode.app and
are used through DEVELOPER_DIR only; xcode-select is never changed.
"""
from __future__ import annotations

import argparse
import base64
import json
import math
import os
import pathlib
import re
import shutil
import subprocess
import sys
import tempfile

ROOT = pathlib.Path(__file__).resolve().parent.parent
BRAND = ROOT / "assets/brand"
ICONS = ROOT / "apps/desktop/src-tauri/icons"
DOCK = ICONS / "dock"
LAYERED = BRAND / "harbard.icon"
XCODE = pathlib.Path(os.environ.get("HARBARD_XCODE", "/Applications/Xcode.app"))
ICTOOL = XCODE / "Contents/Applications/Icon Composer.app/Contents/Executables/ictool"

TILE = "#2a241d"
INK = "#ffffff"
# The presence hues of the console (harbard_core::theme::STATUS).
GREEN = "#2bac76"
AMBER = "#e8a317"
RED = "#d4424a"
GREY = "#a3a1a3"

# The dot on the 64-unit mark grid, and the gap cut around it.
DOT_X, DOT_Y, DOT_R, GAP_R = 54.0, 13.0, 7.0, 10.0

SIZES = (512, 1024)
# Lasting states, by the console's LogoState names. connecting and
# disconnected share the hollow ring: neither has a live bus behind it.
STATES = ("idle", "working", "needs", "blocked", "disconnected")
FRAMES = {"message": 10, "verified": 12}
TRAY = ("dot", "ring", "alert")


def mark_path() -> str:
    raw = (BRAND / "mark.svg").read_text()
    found = re.findall(r' d="([^"]+)"', raw)
    if len(found) != 1:
        sys.exit("render-dock: mark.svg must hold exactly one path")
    return found[0]


def fmt(v: float) -> str:
    return f"{v:.3f}".rstrip("0").rstrip(".")


def dot_layer(state: str) -> tuple[str, float]:
    """The presence mark for a lasting state, on the 64 grid, and its gap."""
    c = f'cx="{fmt(DOT_X)}" cy="{fmt(DOT_Y)}"'
    if state == "idle":
        return f'<circle {c} r="{fmt(DOT_R)}" fill="{GREEN}"/>', GAP_R
    if state == "working":
        # The console's wave from the dot, held still: a halo in a wider gap.
        return (
            f'<circle {c} r="10.4" fill="none" stroke="{GREEN}" stroke-width="1.7" stroke-opacity=".75"/>'
            f'<circle {c} r="{fmt(DOT_R)}" fill="{GREEN}"/>',
            12.8,
        )
    if state == "needs":
        return f'<circle {c} r="{fmt(DOT_R)}" fill="{AMBER}"/>', GAP_R
    if state == "blocked":
        return f'<circle {c} r="{fmt(DOT_R)}" fill="{RED}"/>', GAP_R
    if state == "disconnected":
        return f'<circle {c} r="5.9" fill="none" stroke="{GREY}" stroke-width="2.6"/>', GAP_R
    raise ValueError(state)


def mensaje_frame(i: int, n: int) -> tuple[str, float]:
    """A new message: the dot pops and a ring leaves it, once."""
    t = i / (n - 1)
    grow = math.sin(math.pi * min(t / 0.7, 1.0))
    r = DOT_R * (1 + 0.42 * grow)
    c = f'cx="{fmt(DOT_X)}" cy="{fmt(DOT_Y)}"'
    out = ""
    if 0 < i < n - 1:
        out += (
            f'<circle {c} r="{fmt(DOT_R + 9.5 * t)}" fill="none" stroke="{GREEN}" '
            f'stroke-width="{fmt(2.4 - 1.2 * t)}" stroke-opacity="{fmt(0.9 * (1 - t))}"/>'
        )
    out += f'<circle {c} r="{fmt(r)}" fill="{GREEN}"/>'
    return out, r + 3.0


CHECK = "M50.4 13.3L53 15.9L57.8 10.6"
CHECK_LEN = math.hypot(2.6, 2.6) + math.hypot(4.8, 5.3)


def verificado_frame(i: int, n: int) -> tuple[str, float]:
    """Verified: the dot grows, a check is drawn in it, then it settles."""
    grow_until, draw_until, hold_until = 3, 8, 10
    if i < grow_until:
        k = (i + 1) / grow_until
    elif i < hold_until:
        k = 1.0
    else:
        k = 1 - (i - hold_until + 1) / (n - hold_until + 1)
    r = DOT_R + 2.4 * k
    draw = 0.0 if i < grow_until else min(1.0, (i - grow_until + 1) / (draw_until - grow_until))
    alpha = 1.0 if i < hold_until else k
    c = f'cx="{fmt(DOT_X)}" cy="{fmt(DOT_Y)}"'
    out = f'<circle {c} r="{fmt(r)}" fill="{GREEN}"/>'
    if draw > 0:
        out += (
            f'<path d="{CHECK}" fill="none" stroke="{INK}" stroke-width="2.1" stroke-linecap="round" '
            f'stroke-linejoin="round" stroke-dasharray="{fmt(CHECK_LEN)}" '
            f'stroke-dashoffset="{fmt(CHECK_LEN * (1 - draw))}" opacity="{fmt(alpha)}"/>'
        )
    return out, r + 3.0


def dock_svg(path: str, dot: str, gap: float) -> str:
    """A Dock tile on Apple's macOS grid with the mark at the app icon's scale."""
    inset, body = 100.0, 824.0
    scale = 11.2 * body / 1024
    shift = inset + 153.6 * body / 1024
    return (
        '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 1024 1024" width="1024" height="1024">'
        '<defs><filter id="drop" x="-10%" y="-10%" width="120%" height="125%">'
        '<feGaussianBlur in="SourceAlpha" stdDeviation="9"/><feOffset dy="10" result="b"/>'
        '<feComponentTransfer><feFuncA type="linear" slope=".32"/></feComponentTransfer>'
        '<feMerge><feMergeNode/><feMergeNode in="SourceGraphic"/></feMerge></filter>'
        f'<mask id="gap" maskUnits="userSpaceOnUse" x="0" y="0" width="64" height="64">'
        f'<rect width="64" height="64" fill="#fff"/><circle cx="{fmt(DOT_X)}" cy="{fmt(DOT_Y)}" r="{fmt(gap)}" fill="#000"/></mask></defs>'
        f'<rect x="{fmt(inset)}" y="{fmt(inset)}" width="{fmt(body)}" height="{fmt(body)}" rx="185.4" fill="{TILE}" filter="url(#drop)"/>'
        f'<g transform="translate({fmt(shift)} {fmt(shift)}) scale({fmt(scale)})">'
        f'<path mask="url(#gap)" fill="{INK}" fill-rule="evenodd" d="{path}"/>{dot}</g></svg>'
    )


def tray_svg(path: str, variant: str) -> str:
    c = f'cx="{fmt(DOT_X)}" cy="{fmt(DOT_Y)}"'
    mark = {
        "dot": f'<circle {c} r="{fmt(DOT_R)}" fill="#000000"/>',
        "ring": f'<circle {c} r="5.6" fill="none" stroke="#000000" stroke-width="3.2"/>',
        "alert": '<rect x="51.9" y="3.4" width="4.2" height="11.4" rx="2.1" fill="#000000"/>'
        '<circle cx="54" cy="19.9" r="2.5" fill="#000000"/>',
    }[variant]
    return (
        '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 44 44" width="44" height="44" fill="none">'
        '<!-- macOS template image, generated by scripts/render-dock.py. -->'
        f'<mask id="gap" maskUnits="userSpaceOnUse" x="0" y="0" width="64" height="64"><rect width="64" height="64" fill="#ffffff"/>'
        f'<circle {c} r="{fmt(GAP_R)}" fill="#000000"/></mask>'
        f'<g transform="scale(0.6875)"><path mask="url(#gap)" fill="#000000" fill-rule="evenodd" d="{path}"/>{mark}</g></svg>'
    )


def layer_svgs(path: str) -> dict[str, str]:
    """Icon Composer layers on its 1024 canvas, at the app icon's placement."""
    head = '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 1024 1024" width="1024" height="1024">'
    g = '<g transform="translate(153.6 153.6) scale(11.2)">'
    return {
        "bubble.svg": head
        + f'<mask id="gap" maskUnits="userSpaceOnUse" x="0" y="0" width="64" height="64"><rect width="64" height="64" fill="#ffffff"/>'
        f'<circle cx="{fmt(DOT_X)}" cy="{fmt(DOT_Y)}" r="{fmt(GAP_R)}" fill="#000000"/></mask>'
        + g
        + f'<path mask="url(#gap)" fill="{INK}" fill-rule="evenodd" d="{path}"/></g></svg>\n',
        "dot.svg": head + g + f'<circle cx="{fmt(DOT_X)}" cy="{fmt(DOT_Y)}" r="{fmt(DOT_R)}" fill="{GREEN}"/></g></svg>\n',
    }


def rsvg(svg: str, size: int, out: pathlib.Path, tmp: pathlib.Path) -> None:
    src = tmp / "in.svg"
    src.write_text(svg)
    subprocess.run(["rsvg-convert", "-w", str(size), "-h", str(size), str(src), "-o", str(out)], check=True)


def frames() -> list[tuple[str, str, float]]:
    out = []
    for state in STATES:
        dot, gap = dot_layer(state)
        out.append((state, dot, gap))
    for event, n in FRAMES.items():
        make = mensaje_frame if event == "message" else verificado_frame
        for i in range(n):
            dot, gap = make(i, n)
            out.append((f"{event}-{i:02d}", dot, gap))
    return out


def xcode_env() -> dict[str, str]:
    return {**os.environ, "DEVELOPER_DIR": str(XCODE / "Contents/Developer")}


STAMP = ICONS / "Assets.car.sources"
STAMP_SOURCES = ("icon.json", "Assets/bubble.svg", "Assets/dot.svg")


def layered_stamp() -> str:
    """FNV-1a 64 of the .icon's sources; the same as src/layered_stamp.rs."""
    h = 0xCBF29CE484222325
    for name in STAMP_SOURCES:
        for chunk in (name.encode(), b"\0", (LAYERED / name).read_bytes(), b"\0"):
            for b in chunk:
                h = ((h ^ b) * 0x100000001B3) & 0xFFFFFFFFFFFFFFFF
    return f"fnv1a64:{h:016x}"


def compile_layered(tmp: pathlib.Path) -> str:
    """Assets.car from harbard.icon, or why it could not be compiled."""
    actool = XCODE / "Contents/Developer/usr/bin/actool"
    if not actool.exists():
        return f"actool not found at {actool}"
    out = tmp / "car"
    out.mkdir()
    cmd = [
        str(actool), str(LAYERED), "--compile", str(out),
        "--output-format", "human-readable-text", "--notices", "--warnings", "--errors",
        "--output-partial-info-plist", str(tmp / "partial.plist"),
        "--app-icon", "harbard", "--include-all-app-icons",
        "--enable-on-demand-resources", "NO", "--development-region", "en",
        "--target-device", "mac", "--minimum-deployment-target", "12.0", "--platform", "macosx",
    ]
    run = subprocess.run(cmd, env=xcode_env(), capture_output=True, text=True)
    car = out / "Assets.car"
    if run.returncode != 0 or not car.exists():
        why = (run.stderr or run.stdout).strip().splitlines()
        return "actool failed: " + (why[0] if why else f"exit {run.returncode}")
    shutil.copy2(car, ICONS / "Assets.car")
    STAMP.write_text(layered_stamp() + "\n")
    return ""


APPEARANCES = ("Default", "Dark", "ClearLight", "ClearDark", "TintedLight", "TintedDark")


def appearances(tmp: pathlib.Path, size: int = 256) -> dict[str, pathlib.Path]:
    """The layered icon as macOS 26 draws it in each appearance, by ictool."""
    if not ICTOOL.exists():
        return {}
    out: dict[str, pathlib.Path] = {}
    for rendition in APPEARANCES:
        png = tmp / f"appearance-{rendition}.png"
        cmd = [
            str(ICTOOL), str(LAYERED), "--export-image", "--output-file", str(png),
            "--platform", "macOS", "--rendition", rendition,
            "--width", str(size), "--height", str(size), "--scale", "1",
        ]
        if rendition.startswith("Tinted"):
            cmd += ["--tint-color", "0.83", "--tint-strength", "0.75"]
        run = subprocess.run(cmd, env=xcode_env(), capture_output=True, text=True)
        if run.returncode == 0 and png.exists():
            out[rendition] = png
        else:
            print(f"render-dock: ictool {rendition}: {(run.stderr or run.stdout).strip()}", file=sys.stderr)
    return out


def data_uri(p: pathlib.Path) -> str:
    return "data:image/png;base64," + base64.b64encode(p.read_bytes()).decode()


def html_page(names: list[str], looks: dict[str, pathlib.Path], car_note: str) -> str:
    docks = (("Dock claro", "#e9e7ea", "#1d1c1f"), ("Dock oscuro", "#2a272c", "#ecebee"))
    rows = []
    for title, bg, ink in docks:
        cells = []
        for name in names:
            src = data_uri(DOCK / f"{name}-512.png")
            imgs = "".join(f'<img src="{src}" width="{s}" height="{s}" alt="{name} {s}">' for s in (128, 64, 32))
            cells.append(f'<figure>{imgs}<figcaption>{name}</figcaption></figure>')
        rows.append(f'<section style="background:{bg};color:{ink}"><h2>{title}</h2><div class="grid">{"".join(cells)}</div></section>')
    tray = []
    for title, bg, ink, invert in (("Barra de menús clara", "#f3f4f5", "#20242a", ""), ("Barra de menús oscura", "#20242a", "#e2e4e7", "filter:invert(1)")):
        cells = "".join(
            f'<figure><img src="{data_uri(ICONS / f"tray-{v}.png")}" width="22" height="22" style="{invert}" alt="{v}"><figcaption>{v}</figcaption></figure>'
            for v in TRAY
        )
        tray.append(f'<section style="background:{bg};color:{ink}"><h2>{title} (22 pt)</h2><div class="grid">{cells}</div></section>')
    look_cells = "".join(
        f'<figure><img src="{data_uri(p)}" width="128" height="128" alt="{r}"><figcaption>{r}</figcaption></figure>'
        for r, p in looks.items()
    ) or "<p>ictool no está disponible.</p>"
    players = []
    for event, n in FRAMES.items():
        srcs = [data_uri(DOCK / f"{event}-{i:02d}-512.png") for i in range(n)]
        players.append(
            f'<figure class="play" data-frames=\'{json.dumps(srcs)}\'><img src="{srcs[-1]}" width="128" height="128" alt="{event}">'
            f'<figcaption><button type="button">Reproducir {event}</button></figcaption></figure>'
        )
    return f"""<!doctype html><html lang="es"><meta charset="utf-8"><title>harbard · icono del Dock</title>
<style>
body{{margin:0;font:14px/1.4 -apple-system,Helvetica,sans-serif;background:#111;color:#eee}}
section{{padding:16px 20px}} h1{{padding:16px 20px 0;margin:0;font-size:20px}} h2{{font-size:15px;margin:0 0 10px}}
.grid{{display:flex;flex-wrap:wrap;gap:14px}} figure{{margin:0;display:flex;flex-direction:column;align-items:center;gap:4px}}
figure img{{display:block}} figcaption{{font-size:12px;opacity:.8}} p.note{{padding:0 20px;opacity:.8}}
</style>
<h1>harbard · icono vivo del Dock</h1>
<p class="note">Cada estado y cada fotograma prerenderizado, a 128, 64 y 32 px. Generado por scripts/render-dock.py.</p>
{"".join(rows)}
<section style="background:#2a272c"><h2>Animaciones de una vez (fotogramas a 80 ms, una sola pasada)</h2><div class="grid">{"".join(players)}</div></section>
{"".join(tray)}
<section style="background:#1b1a1d"><h2>Icono por capas de macOS 26, renderizado con ictool</h2><div class="grid">{look_cells}</div>
<p>{car_note}</p></section>
<script>
for (const fig of document.querySelectorAll('.play')) {{
  const frames = JSON.parse(fig.dataset.frames), img = fig.querySelector('img');
  fig.querySelector('button').onclick = () => {{
    let i = 0; const step = () => {{ img.src = frames[i++]; if (i < frames.length) setTimeout(step, 80); }}; step();
  }};
}}
</script></html>
"""


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("--html", type=pathlib.Path)
    args = ap.parse_args()
    if not shutil.which("rsvg-convert"):
        sys.exit("missing: rsvg-convert")
    path = mark_path()
    DOCK.mkdir(parents=True, exist_ok=True)
    (LAYERED / "Assets").mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory() as t:
        tmp = pathlib.Path(t)
        names = []
        for name, dot, gap in frames():
            names.append(name)
            svg = dock_svg(path, dot, gap)
            for size in SIZES:
                rsvg(svg, size, DOCK / f"{name}-{size}.png", tmp)
        for variant in TRAY:
            rsvg(tray_svg(path, variant), 44, ICONS / f"tray-{variant}.png", tmp)
        for name, svg in layer_svgs(path).items():
            (LAYERED / "Assets" / name).write_text(svg)
        why = compile_layered(tmp)
        stale = bool(why) and (not STAMP.exists() or STAMP.read_text().strip() != layered_stamp())
        if stale:
            # macOS draws the car, not the .icns, wherever CFBundleIconName is
            # set: a car older than its sources must not be kept silently.
            sys.exit(f"render-dock: icons/Assets.car is stale and could not be rebuilt ({why})")
        if why:
            note = f"Assets.car no se ha compilado: {why}. El paquete conserva el .icns."
            print(f"render-dock: layered icon not compiled ({why}); the bundle keeps icon.icns", file=sys.stderr)
        else:
            note = "Assets.car compilado con actool."
            print("render-dock: compiled icons/Assets.car")
        looks = appearances(tmp) if args.html else {}
        if args.html:
            args.html.write_text(html_page(names, looks, note))
            for rendition, png in looks.items():
                shutil.copy2(png, args.html.parent / f"harbard-dock-{rendition}.png")
            print(f"render-dock: wrote {args.html}")
    print(f"render-dock: {len(names)} Dock images at {SIZES}, {len(TRAY)} tray variants")


if __name__ == "__main__":
    main()
