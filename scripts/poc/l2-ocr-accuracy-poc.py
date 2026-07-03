#!/usr/bin/env python3
"""
L2 OCR 准确率 POC(屏幕记忆,见 docs/design/screen-memory.md)。

完全受控实验:自己渲染"模拟屏幕"(已知文字 = ground truth),测
「文字物理像素高度 × 存档宽度」矩阵下的 Vision OCR 字符准确率。

背景:存档管线把屏幕缩到 1280 宽(JPEG q80)。同样 13px 的界面文字,
在 2× Retina 屏上有 26 物理像素,缩到 1280 还剩 13px → OCR 没问题;
在 1× QHD(2560×1440 原生渲染)上只有 13 物理像素,缩完剩 6.5px → OCR 报废。
本脚本量化这条曲线,决定 L2 该在什么分辨率上跑 / 存档参数怎么改。

用法:python3 scripts/poc/l2-ocr-accuracy-poc.py
输出:准确率矩阵(stdout)+ 模拟屏幕与压缩样张(scripts/poc/output/,已 gitignore)。
依赖:PIL;首次运行自动编译 scripts/poc/ocr-cli.swift。
"""
import os, re, subprocess, sys
from collections import Counter
from PIL import Image, ImageDraw, ImageFont

# Vision 框架在部分机器上向 stdout 吐模型警告(E5 bundle),会污染识别文本,精确滤除
_E5_NOISE = re.compile(
    r"Unable to find a valid E5 in provided path.*?GetE5PathFromCompositeBundle",
    re.DOTALL)

HERE = os.path.dirname(os.path.abspath(__file__))
OUT = os.path.join(HERE, "output", "l2")
CLI = os.path.join(HERE, "output", "ocr-cli")  # 共用工具放 output 根

CANVAS_W, CANVAS_H = 2560, 1440          # 模拟一块 QHD 物理屏
FONT_PX = [10, 12, 13, 14, 16, 20, 26]   # 文字物理像素高(13=QHD 1× 常见 UI;26=Retina 2×)
STORE_W = [1280, 1600, 1920, 2560]       # 存档宽度(2560=不缩)
JPEG_Q = 80                              # 与 app 现状一致

def load_font(size):
    for p in ["/System/Library/Fonts/PingFang.ttc",
              "/System/Library/Fonts/Hiragino Sans GB.ttc",
              "/Library/Fonts/Arial Unicode.ttf"]:
        try:
            return ImageFont.truetype(p, size)
        except Exception:
            continue
    sys.exit("找不到 CJK 字体")

def make_screen(px):
    """渲染一屏已知内容:混合中英数字,模拟真实界面文本密度。返回 (img, ground_truth)。"""
    img = Image.new("RGB", (CANVAS_W, CANVAS_H), (250, 250, 250))
    d = ImageDraw.Draw(img)
    f = load_font(px)
    lines, y, i = [], int(px * 1.5), 1
    while y < CANVAS_H - px * 2:
        t = (f"{i:02d} 订单2026-{i:04d} 已支付¥{199+i}.00 Keychron K8机械键盘 "
             f"quantity:{i} status=shipped 富士山旅行攻略第{i}章")
        d.text((40, y), t, font=f, fill=(25, 25, 25))
        lines.append(t)
        y += int(px * 1.9)
        i += 1
    return img, "".join(lines)

def ocr(path):
    r = subprocess.run([CLI, path], capture_output=True, text=True, timeout=120)
    return _E5_NOISE.sub("", r.stdout)

def norm(s):
    return "".join(s.split())  # 去所有空白后逐字符比

def accuracy(gt, got):
    """字符召回率:识别文本覆盖了 ground truth 多少字符(multiset 交集,不看顺序)。
    difflib.SequenceMatcher 不可用——autojunk 启发式在高重复长文本上会把常见字
    全部当垃圾,比率直接崩到 0。"""
    a, b = Counter(norm(gt)), Counter(norm(got))
    hit = sum((a & b).values())
    return hit / max(1, sum(a.values()))

def main():
    os.makedirs(OUT, exist_ok=True)
    if not os.path.exists(CLI):
        print("编译 ocr-cli …")
        subprocess.run(["swiftc", "-O", os.path.join(HERE, "ocr-cli.swift"), "-o", CLI],
                       check=True, capture_output=True)

    print(f"模拟屏:{CANVAS_W}x{CANVAS_H}(QHD 1×)  JPEG q{JPEG_Q}  存档宽度:{STORE_W}")
    header = "文字物理px(缩到1280后)" + "".join(f"{w:>9}" for w in STORE_W)
    print(header)
    for px in FONT_PX:
        screen, gt = make_screen(px)
        row = []
        for w in STORE_W:
            r = w / CANVAS_W
            v = screen.resize((w, int(CANVAS_H * r)), Image.LANCZOS)
            p = os.path.join(OUT, f"l2_{px}px_{w}w.jpg")
            v.save(p, quality=JPEG_Q)
            row.append(accuracy(gt, ocr(p)))
        eff = px * STORE_W[0] / CANVAS_W
        cells = "".join(f"{a*100:>8.1f}%" for a in row)
        print(f"{px:>4}px ({eff:>4.1f}px)      {cells}")
    print(f"\n样张:{OUT}/l2_<字号>px_<宽>w.jpg(可开图目检)")

if __name__ == "__main__":
    main()
