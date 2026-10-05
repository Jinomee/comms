"""Render the recorded comms session (cast.json) into terminal-style frames.

Commands and outputs come verbatim from record.py's real run. The only
change is shortening the temporary project path to ~/shop-api.
"""
import glob
import json
import os
import re
import sys
import textwrap

from PIL import Image, ImageDraw, ImageFont

HERE = os.path.dirname(os.path.abspath(__file__))
FRAMES = os.path.join(HERE, "frames")
MEDIA = os.path.abspath(os.path.join(HERE, "..", "..", "docs", "media"))
FPS = 12

W, H = 1200, 680
PAD = 28
TITLE_H = 36
HEADER_H = 86
FONT = "/usr/share/fonts/truetype/dejavu/DejaVuSansMono.ttf"
FONT_B = "/usr/share/fonts/truetype/dejavu/DejaVuSansMono-Bold.ttf"
SANS = "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf"
SANS_B = "/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf"

mono = ImageFont.truetype(FONT, 17)
mono_b = ImageFont.truetype(FONT_B, 17)
head = ImageFont.truetype(SANS_B, 26)
sub = ImageFont.truetype(SANS, 18)
small = ImageFont.truetype(SANS, 15)
big = ImageFont.truetype(SANS_B, 54)

BG = (13, 17, 23)
WIN = (22, 27, 34)
BAR = (33, 38, 45)
FG = (230, 237, 243)
DIM = (139, 148, 158)
ACCENT = (88, 166, 255)
WHO_COLOR = {"you": (88, 166, 255), "luna": (63, 185, 80), "nova": (210, 168, 255)}
RED = (248, 81, 73)

CHAR_W = mono.getbbox("M")[2]
LINE_H = 24
TERM_TOP = PAD + TITLE_H + HEADER_H + 14
TERM_LEFT = PAD + 22
TERM_COLS = (W - 2 * PAD - 44) // CHAR_W
TERM_ROWS = (H - TERM_TOP - PAD - 14) // LINE_H

frame_no = 0


def emit(img, seconds):
    global frame_no
    for _ in range(max(1, round(seconds * FPS))):
        img.save(os.path.join(FRAMES, f"f{frame_no:05d}.png"))
        frame_no += 1


def window(draw):
    draw.rectangle([0, 0, W, H], fill=BG)
    draw.rounded_rectangle([PAD, PAD, W - PAD, H - PAD], 12, fill=WIN)
    draw.rounded_rectangle([PAD, PAD, W - PAD, PAD + TITLE_H], 12, fill=BAR)
    draw.rectangle([PAD, PAD + TITLE_H - 12, W - PAD, PAD + TITLE_H], fill=BAR)
    for i, c in enumerate([(255, 95, 86), (255, 189, 46), (39, 201, 63)]):
        x = PAD + 20 + i * 22
        draw.ellipse([x, PAD + 12, x + 12, PAD + 24], fill=c)
    label = "comms demo · ~/shop-api"
    tw = draw.textlength(label, font=small)
    draw.text(((W - tw) / 2, PAD + 9), label, fill=DIM, font=small)


def header(draw, idx, total, title, subtitle):
    y = PAD + TITLE_H + 16
    step = f"{idx}/{total}"
    draw.text((TERM_LEFT, y + 2), step, fill=ACCENT, font=sub)
    sx = TERM_LEFT + draw.textlength(step, font=sub) + 14
    draw.text((sx, y - 2), title, fill=FG, font=head)
    draw.text((sx, y + 34), subtitle, fill=DIM, font=sub)
    draw.line([TERM_LEFT, PAD + TITLE_H + HEADER_H, W - PAD - 22, PAD + TITLE_H + HEADER_H], fill=BAR, width=2)


def prompt_parts(who):
    return [(f"{who} ", WHO_COLOR[who], True), ("~/shop-api $ ", DIM, False)]


def wrap(text, width):
    out = []
    for line in text.split("\n"):
        out.extend(textwrap.wrap(line, width, drop_whitespace=False, replace_whitespace=False) or [""])
    return out


def render(lines, idx, total, title, subtitle, cursor=False):
    img = Image.new("RGB", (W, H))
    d = ImageDraw.Draw(img)
    window(d)
    header(d, idx, total, title, subtitle)
    visible = lines[-TERM_ROWS:]
    for row, parts in enumerate(visible):
        x = TERM_LEFT
        y = TERM_TOP + row * LINE_H
        for text, color, bold in parts:
            d.text((x, y), text, fill=color, font=mono_b if bold else mono)
            x += d.textlength(text, font=mono_b if bold else mono)
        if cursor and row == len(visible) - 1:
            d.rectangle([x + 1, y + 2, x + CHAR_W - 1, y + LINE_H - 4], fill=FG)
    return img


def card(lines_spec, seconds):
    img = Image.new("RGB", (W, H), BG)
    d = ImageDraw.Draw(img)
    total_h = sum(h for _, _, _, h in lines_spec)
    y = (H - total_h) / 2
    for text, font, color, h in lines_spec:
        tw = d.textlength(text, font=font)
        d.text(((W - tw) / 2, y), text, fill=color, font=font)
        y += h
    emit(img, seconds)


def main():
    os.makedirs(FRAMES, exist_ok=True)
    for old in glob.glob(os.path.join(FRAMES, "f*.png")):
        os.remove(old)
    scenes = json.load(open(os.path.join(HERE, "cast.json")))
    project_re = re.compile(r"/\S*?/shop-api")

    card([
        ("comms", big, FG, 78),
        ("Let your coding agents talk to each other.", head, FG, 44),
        ("Second opinions · file claims · rooms · loop limits", sub, DIM, 46),
        ("Every command in this demo ran for real; luna and nova are two agent shells", small, DIM, 22),
        ("running the same commands Claude Code and Codex run through comms.", small, DIM, 22),
    ], 4.0)

    total = len(scenes)
    for idx, scene in enumerate(scenes, 1):
        lines = []
        title, subtitle = scene["title"], scene["subtitle"]
        for step in scene["steps"]:
            parts = prompt_parts(step["who"])
            cmd = step["cmd"]
            typed = ""
            for i, ch in enumerate(cmd):
                typed += ch
                if i % 3 == 2 or i == len(cmd) - 1:  # ~36 chars/s at 12 fps
                    frame_lines = lines + [parts + [(typed, FG, False)]]
                    emit(render(frame_lines, idx, total, title, subtitle, cursor=True), 1 / FPS)
            lines.append(parts + [(cmd, FG, False)])
            emit(render(lines, idx, total, title, subtitle), 0.35)

            if "comms ask" in cmd:
                for i in range(18):
                    spin = "⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏"[i % 10]
                    emit(render(lines + [[(f"{spin} asking a fresh claude (read-only)…", DIM, False)]],
                                idx, total, title, subtitle), 1 / FPS)

            out = project_re.sub("~/shop-api", step["out"]).strip("\n")
            color = RED if step["code"] != 0 else FG
            for wl in wrap(out, TERM_COLS):
                lines.append([(wl, color, False)])
                emit(render(lines, idx, total, title, subtitle), 0.06)
            if step["code"] != 0:
                lines.append([(f"(exit {step['code']}: claimed by someone else)", DIM, False)])
            lines.append([])
            emit(render(lines, idx, total, title, subtitle), 1.6 if "comms ask" not in cmd else 5.5)
        emit(render(lines, idx, total, title, subtitle), 1.4)

    card([
        ("comms", big, FG, 78),
        ("comms claude   ·   comms codex   ·   comms ask", head, FG, 50),
        ("cargo install --path .", sub, ACCENT, 32),
        ("github.com/jinomee/comms", sub, DIM, 30),
    ], 3.5)
    print("frames:", frame_no, "seconds:", round(frame_no / FPS, 1))
    encode()


def encode():
    import subprocess

    os.makedirs(MEDIA, exist_ok=True)
    src = ["-framerate", str(FPS), "-i", os.path.join(FRAMES, "f%05d.png")]
    palette = os.path.join(FRAMES, "palette.png")
    ff = ["ffmpeg", "-loglevel", "error", "-y"]
    scale = "scale=960:-1:flags=lanczos"
    subprocess.run(ff + src + ["-vf", f"{scale},palettegen=max_colors=128:stats_mode=full", palette], check=True)
    subprocess.run(
        ff + src + ["-i", palette, "-lavfi",
                    f"{scale}[x];[x][1:v]paletteuse=dither=none:diff_mode=rectangle",
                    "-loop", "0", os.path.join(MEDIA, "demo.gif")],
        check=True,
    )
    subprocess.run(
        ff + src + ["-c:v", "libx264", "-pix_fmt", "yuv420p", "-crf", "24",
                    "-movflags", "+faststart", os.path.join(MEDIA, "demo.mp4")],
        check=True,
    )
    print("wrote", MEDIA)


if __name__ == "__main__":
    sys.exit(main())
