"""Annotations des captures du mode d'emploi : cadres orange, pastilles numérotées,
fond assombri autour des zones expliquées. Écrit aussi les images du README.

Lance d'abord captures.py.
    python docs/tools/annoter.py
"""
import json, pathlib
from PIL import Image, ImageDraw, ImageFont

ROOT = pathlib.Path(__file__).resolve().parents[2]
OUT = ROOT / "docs" / "tools" / "out"
GUIDE = ROOT / "docs" / "guide"
SHOTS = ROOT / "docs" / "screenshots"
ORANGE = (240, 166, 64)
FONT = "/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf"
GUIDE.mkdir(exist_ok=True)
SHOTS.mkdir(exist_ok=True)

def union(boxes):
    return [min(b[0] for b in boxes), min(b[1] for b in boxes), max(b[2] for b in boxes), max(b[3] for b in boxes)]

def frame(d, b, n, W, H, s=1.0, pad=8, where=None):
    p = pad * s
    x0, y0, x1, y1 = max(b[0] - p, 3), max(b[1] - p, 3), min(b[2] + p, W - 4), min(b[3] + p, H - 4)
    d.rounded_rectangle((x0, y0, x1, y1), radius=int(14 * s), outline=ORANGE, width=max(4, int(5 * s)))
    if not n:
        return
    r = int(24 * s)
    # Pastille au milieu du bord droit (ou gauche), jamais sur une zone voisine.
    if where == "left":
        cx, cy = max(x0, r + 6), (y0 + y1) / 2
    else:
        cx, cy = min(x1, W - r - 6), (y0 + y1) / 2
    d.ellipse((cx - r, cy - r, cx + r, cy + r), fill=ORANGE)
    f = ImageFont.truetype(FONT, int(28 * s))
    t = str(n)
    d.text((cx - d.textlength(t, font=f) / 2, cy - int(17 * s)), t, font=f, fill=(20, 16, 5))

def dim(img, boxes, pad=8, alpha=115):
    mask = Image.new("L", img.size, alpha)
    md = ImageDraw.Draw(mask)
    for x0, y0, x1, y1 in boxes:
        md.rounded_rectangle((x0 - pad, y0 - pad, x1 + pad, y1 + pad), 14, fill=0)
    return Image.composite(Image.new("RGB", img.size, (0, 0, 0)), img, mask)

def box(R, key):
    return R[key] if isinstance(key, str) else key(R)

def make(out, src, lang, marks, crop=None, margin=40, dimbg=True, maxw=1400):
    """marks : (zone, numéro ou None[, marge][, "left"])."""
    img = Image.open(OUT / f"{src}_{lang}.png").convert("RGB")
    R = json.load(open(OUT / f"{src}_{lang}.json"))
    boxes = [(list(box(R, m[0])), m[1], m[2] if len(m) > 2 else 8, m[3] if len(m) > 3 else None) for m in marks]
    if dimbg and not crop:
        img = dim(img, [b[0] for b in boxes])
    s = 1.0
    if crop:
        c = union([box(R, k) for k in crop])
        c = [max(0, c[0] - margin), max(0, c[1] - margin), min(img.width, c[2] + margin), min(img.height, c[3] + margin)]
        img = img.crop(c)
        s = min(2.0, maxw / img.width)
        img = img.resize((int(img.width * s), int(img.height * s)), Image.LANCZOS)
        boxes = [([(b[0] - c[0]) * s, (b[1] - c[1]) * s, (b[2] - c[0]) * s, (b[3] - c[1]) * s], n, p, w) for b, n, p, w in boxes]
    d = ImageDraw.Draw(img)
    for b, n, p, w in boxes:
        frame(d, b, n, img.width, img.height, s=1.0 if not crop else max(1.0, s * 0.8), pad=p, where=w)
    if img.width > 1440:
        img = img.resize((1440, round(img.height * 1440 / img.width)), Image.LANCZOS)
    img.quantize(256, method=Image.Quantize.MEDIANCUT, dither=Image.Dither.NONE).save(GUIDE / f"{out}_{lang}.png", optimize=True)

def readme(name, src, lang):
    img = Image.open(OUT / f"{src}_{lang}.png").convert("RGB").resize((1600, 1025), Image.LANCZOS)
    img.save(SHOTS / f"{name}_{lang}.webp", "WEBP", quality=88, method=6)

share = lambda R: union([R["shareLink"], R["copyRow"]])
code = lambda R: union([R["shareCode"], R["newCode"]])
top = lambda R: union([R["verBadge"], R["lang"], R["settings"], R["about"]])
bars = lambda R: union([R["pct"], R["bar"]])

for L in ["fr", "en", "es"]:
    make("00_welcome", "welcome", L, [("warnbox", 1), ("modalOk", 2, 8, "left")], dimbg=False)
    make("01_interface", "send_empty", L, [("modes", 1), ("drop", 2), ("picks", 3), ("after", 4), ("go", 5, 8, "left"), (top, 6, 30, "left")])
    make("02_choose_send", "send_choose", L, [("tools", 1), ("tree", 2), ("sum", 3), ("drop", 4)])
    make("03_publishing", "send_publishing", L, [("steps", 1), ("phase", 2), ("pub", 3)], crop=["status", "share"], margin=30)
    make("04_link", "send_link", L, [("pub", 1), (share, 2), (code, 3), ("tip", 4)], crop=["share"], margin=30)
    make("05_send_progress", "send_progress", L, [(bars, 1), ("vtrack", 2), ("metrics", 3), ("log", 4), ("stop", 5, 8, "left")])
    make("06_send_verify", "send_verify", L, [("phase", 1), ("vtrack", 2)])
    make("07_delivered", "send_done", L, [("steps", 1), ("banner", 2)])
    make("08_receive_form", "recv_form", L, [("link", 1), ("code", 2), ("dest", 3), ("night", 4), ("go", 5, 8, "left")])
    make("09_receive_choose", "recv_choose", L, [("phase", 1), ("tools", 2), ("tree", 3), ("pickOk", 4, 8, "left")])
    make("10_receive_progress", "recv_progress", L, [(bars, 1), ("vtrack", 2), ("files", 3)])
    make("11_receive_verify", "recv_verify", L, [("phase", 1), ("vtrack", 2)])
    make("12_receive_mhl", "recv_mhl", L, [("steps", 1), ("phase", 2)], crop=["status"], margin=30)
    make("13_received", "recv_done", L, [("banner", 1), ("bannerBtns", 2), ("files", 3)])
    make("14_noctiluca", "recv_night", L, [("night", 1), ("steps", 2), ("phase", 3)])
    make("15_update", "update", L, [("verBadge", None, 6)], crop=["verBadge", "about"], margin=24, maxw=900)
    make("16_settings", "settings", L, [("optUpdate", 1, 8)], crop=["modal"], margin=24)
    readme("link", "send_link", L)
    readme("choose", "recv_choose", L)
    readme("received", "recv_done", L)
print("images écrites dans", GUIDE, "et", SHOTS)
