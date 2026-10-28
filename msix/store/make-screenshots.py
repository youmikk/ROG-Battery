# -*- coding: utf-8 -*-
r"""把 scratch 里的界面渲染图合成成 Microsoft Store 要的桌面截图（1920x1080）。

Store 要求：桌面截图至少 1366x768，PNG/JPG，单张 ≤10 MB。程序原生窗口是
1100x740，直接上传尺寸不够，所以统一贴到 1920x1080 画布上：柔和渐变底 + 圆角
窗口 + 投影。图片本身不做任何内容改动（不 P 数值、不改文案），只是排版。

用法：python msix\store\make-screenshots.py
"""
import os

from PIL import Image, ImageDraw, ImageFilter

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(os.path.dirname(HERE))
SCRATCH = os.path.join(ROOT, "scratch")
OUTDIR = os.path.join(HERE, "screenshots")

CANVAS = (1920, 1080)
SHOT_H = 940          # 窗口在画布上的高度
RADIUS = 14           # 圆角
SHADOW_ALPHA = 78
SHADOW_BLUR = 26
SHADOW_DY = 18

# (源图, 输出名, 明/暗底)
SHOTS = [
    ("battery-eta-live.png", "01-battery.png", "light"),
    ("ui-perf-top.png", "02-performance.png", "light"),
    ("ui-keys.png", "03-buttons.png", "light"),
    ("ui-light-dyn.png", "04-lighting.png", "light"),
    ("ui-settings-top.png", "05-settings.png", "light"),
    # 深色主题示例用性能页：四档彩色指示灯 + 药丸分段控件，比按键页好看，也不会
    # 和 03 重复同一页面。电量页的测试 fixture 电压是 0 mV、历史 0 个样本，像坏界面，不能用。
    ("ui-perf-dark.png", "06-dark-theme.png", "dark"),
]


def gradient(size, top, bottom):
    w, h = size
    img = Image.new("RGB", (1, h))
    d = ImageDraw.Draw(img)
    for y in range(h):
        t = y / max(h - 1, 1)
        d.point((0, y), tuple(int(top[i] * (1 - t) + bottom[i] * t) for i in range(3)))
    return img.resize((w, h), Image.BILINEAR)


def backdrop(kind):
    if kind == "dark":
        base = gradient(CANVAS, (30, 34, 42), (12, 14, 18))
        glow = (76, 194, 255)
    else:
        base = gradient(CANVAS, (247, 250, 253), (224, 231, 240))
        glow = (0, 120, 212)

    # 中间一团很淡的径向光，避免整块死平
    halo = Image.new("L", CANVAS, 0)
    hd = ImageDraw.Draw(halo)
    w, h = CANVAS
    hd.ellipse([w * 0.12, h * 0.05, w * 0.88, h * 0.95], fill=90)
    halo = halo.filter(ImageFilter.GaussianBlur(180))
    tint = Image.new("RGB", CANVAS, glow)
    base = Image.composite(Image.blend(base, tint, 0.16), base, halo)
    return base.convert("RGBA")


def rounded(img, radius):
    mask = Image.new("L", img.size, 0)
    ImageDraw.Draw(mask).rounded_rectangle([0, 0, img.size[0] - 1, img.size[1] - 1], radius=radius, fill=255)
    out = img.copy()
    out.putalpha(mask)
    return out


def compose(src_name, out_name, kind):
    src = os.path.join(SCRATCH, src_name)
    win = Image.open(src).convert("RGBA")
    s = SHOT_H / win.height
    win = win.resize((int(win.width * s), SHOT_H), Image.LANCZOS)
    win = rounded(win, RADIUS)

    canvas = backdrop(kind)
    x = (CANVAS[0] - win.width) // 2
    y = (CANVAS[1] - win.height) // 2

    # 投影：窗口 alpha 当形状，模糊后压暗
    shadow = Image.new("RGBA", CANVAS, (0, 0, 0, 0))
    plate = Image.new("RGBA", win.size, (0, 0, 0, SHADOW_ALPHA))
    plate.putalpha(win.split()[3].point(lambda a: a * SHADOW_ALPHA // 255))
    shadow.paste(plate, (x, y + SHADOW_DY), plate)
    shadow = shadow.filter(ImageFilter.GaussianBlur(SHADOW_BLUR))

    canvas = Image.alpha_composite(canvas, shadow)
    canvas.paste(win, (x, y), win)

    os.makedirs(OUTDIR, exist_ok=True)
    out = os.path.join(OUTDIR, out_name)
    canvas.convert("RGB").save(out, "PNG", optimize=True)
    print("%-22s <- %-26s %dx%d  %8d bytes" % (out_name, src_name, CANVAS[0], CANVAS[1], os.path.getsize(out)))


def main():
    for src, out, kind in SHOTS:
        if not os.path.exists(os.path.join(SCRATCH, src)):
            print("跳过 %s（缺 %s）" % (out, src))
            continue
        compose(src, out, kind)


if __name__ == "__main__":
    main()
