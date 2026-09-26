# アイコンを描き直す: icon-src の SVG 2 枚と、そこから描く PNG 2 枚(cockpit-icon.png・
# Cockpit.icon の層)を書き出す。rsvg-convert が要る(brew install librsvg)。
#
#   python3 rust/crates/armature/assets/icon-src/make-icon.py
import json
import pathlib
import subprocess

SRC = pathlib.Path(__file__).resolve().parent
ASSETS = SRC.parent

# 細い >_ と、時計と同じ淡い光。
STROKE = 22
INK = "#e2e7fa"
GLOW = (
    '<filter id="glow" x="-50%" y="-50%" width="200%" height="200%">'
    '<feGaussianBlur in="SourceGraphic" stdDeviation="14" result="b"/>'
    '<feColorMatrix in="b" type="matrix" values="1 0 0 0 0  0 1 0 0 0  0 0 1 0 0  0 0 0 .55 0" result="g"/>'
    '<feMerge><feMergeNode in="g"/><feMergeNode in="SourceGraphic"/></feMerge></filter>'
)
TOP, BOTTOM = "#1e2030", "#15161e"


def prompt(cx=512, cy=512, h=250, w=125):
    """> の高さ h・幅 w。_ は > の下端に揃える。"""
    x0, y0 = cx - w / 2 - h * 0.35, cy - h / 2
    ux = x0 + w + h * 0.26
    return (
        f'<polyline points="{x0:.1f},{y0:.1f} {x0 + w:.1f},{cy:.1f} {x0:.1f},{y0 + h:.1f}" fill="none" '
        f'stroke="{INK}" stroke-width="{STROKE}" stroke-linecap="round" stroke-linejoin="round"/>'
        f'<line x1="{ux:.1f}" y1="{y0 + h:.1f}" x2="{ux + h * 0.52:.1f}" y2="{y0 + h:.1f}" '
        f'stroke="{INK}" stroke-width="{STROKE}" stroke-linecap="round"/>'
    )


# 角丸の地まで描いた 1 枚(Icon Composer の無い機体で使う)。
FULL = (
    '<svg xmlns="http://www.w3.org/2000/svg" width="1024" height="1024" viewBox="0 0 1024 1024">\n'
    f'  <defs><linearGradient id="bg" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="{TOP}"/>'
    f'<stop offset="1" stop-color="{BOTTOM}"/></linearGradient>'
    f'<clipPath id="sq"><rect x="100" y="100" width="824" height="824" rx="185"/></clipPath>{GLOW}</defs>\n'
    '  <g clip-path="url(#sq)"><rect x="100" y="100" width="824" height="824" fill="url(#bg)"/>'
    f'<g filter="url(#glow)">{prompt()}</g></g>\n'
    '  <rect x="101.5" y="101.5" width="821" height="821" rx="183.5" fill="none" stroke="#ffffff" '
    'stroke-opacity=".07" stroke-width="3"/>\n'
    '</svg>\n'
)

# Icon Composer の層。層の四角の全面が角丸の地に当たるので、824 → 1024 に広げる。
LAYER = (
    '<svg xmlns="http://www.w3.org/2000/svg" width="1024" height="1024" viewBox="0 0 1024 1024">\n'
    f'  <defs>{GLOW}</defs>\n'
    '  <g transform="translate(512 512) scale(1.2427) translate(-512 -512)">'
    f'<g filter="url(#glow)">{prompt()}</g></g>\n'
    '</svg>\n'
)


def srgb(hex_color):
    r, g, b = (int(hex_color[i : i + 2], 16) / 255 for i in (1, 3, 5))
    return f"srgb:{r:.5f},{g:.5f},{b:.5f},1.00000"


def render(svg, png):
    subprocess.run(["rsvg-convert", "-w", "1024", "-h", "1024", str(svg), "-o", str(png)], check=True)


(SRC / "icon.svg").write_text(FULL)
(SRC / "icon-layer.svg").write_text(LAYER)
render(SRC / "icon.svg", ASSETS / "cockpit-icon.png")
render(SRC / "icon-layer.svg", ASSETS / "Cockpit.icon/Assets/prompt.png")
spec_path = ASSETS / "Cockpit.icon/icon.json"
spec = json.loads(spec_path.read_text())
spec["fill"] = {"linear-gradient": [srgb(TOP), srgb(BOTTOM)]}
spec_path.write_text(json.dumps(spec, indent=2) + "\n")
