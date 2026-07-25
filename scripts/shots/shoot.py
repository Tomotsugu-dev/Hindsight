#!/usr/bin/env python3
"""README 界面截图一键更新(macOS)。

流程:你手动起好演示实例并切好界面语言 → 跑本脚本 → 它用 HumanMoveMouse
逐页导航、按窗口截图,把 8 张图直接写进对应语言的 imgs 目录。

    # 1) 起演示实例(数据先生成好),在应用里切到目标语言
    npm run demo
    # 2) 跑一种语言的全套截图(uv 会按下方内联声明自动备好依赖)
    uv run scripts/shots/shoot.py --lang zh
    # 3) 在应用里切下一种语言,再跑
    uv run scripts/shots/shoot.py --lang en

坐标校准(换电脑 / 改窗口大小 / UI 改版后跑一次):
    uv run scripts/shots/shoot.py --where
    # 鼠标悬停到目标上,终端每 0.5 秒打印一行「窗口相对坐标」,
    # 把数字填进下面的 COORDS 即可。所有点击坐标都是相对应用窗口左上角的,
    # 窗口挪动不影响,窗口"缩放"才需要重校。

依赖与权限:
  - 用 uv 跑零配置;不用 uv 的话:pip3 install HumanMoveMouse(需 Python ≥3.10,
    系统自带的 3.9 编译 pyobjc 会失败)
  - 系统设置 → 隐私与安全性 → 辅助功能:勾选你跑脚本的终端(驱动鼠标)
  - 系统设置 → 隐私与安全性 → 屏幕录制:勾选同一个终端(窗口截图)

注意:
  - 截图期间别碰鼠标键盘;整套约 40 秒。
  - 左上角账号邮箱默认自动遮蔽(侧边栏底色圆角块),--no-mask 关闭。
  - cloud_sync 在演示库里是"未登录"状态,想要"已连接"效果就用真实实例
    手动截那一张(记得打码邮箱),或 --only 排除它。
"""

# /// script
# requires-python = ">=3.10"
# dependencies = ["HumanMoveMouse", "pillow"]
# ///

import argparse
import subprocess
import sys
import time
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]

# ───────────────────────── 配置 ─────────────────────────

LANG_DIRS = {
    "zh": REPO / "docs/intro_zh/imgs",
    "zh-tw": REPO / "docs/intro_zh-TW/imgs",
    "en": REPO / "docs/intro_en/imgs",
    "ja": REPO / "docs/intro_ja/imgs",
    "pt": REPO / "docs/intro_pt/imgs",
}

# 点击坐标,全部是【窗口相对】像素(逻辑分辨率,非 Retina 物理像素)。
# 按 960×720 窗口从实拍图量出;改窗口大小后跑 --where 重校。
COORDS = {
    "nav_daily":     (70, 165),   # 侧边栏:日统计 / Daily
    "nav_weekly":    (70, 203),   # 侧边栏:周统计 / Weekly
    "nav_monthly":   (70, 241),   # 侧边栏:月统计 / Monthly
    "nav_chat":      (70, 313),   # 侧边栏:对话 / Chat
    "nav_summary":   (70, 351),   # 侧边栏:AI 总结 / AI Summary
    "nav_cloud":     (70, 537),   # 侧边栏:云同步 / Cloud Sync
    "tab_ratio":     (330, 157),  # 月统计页:「占比 / Share」tab
    "tab_hours":     (250, 157),  # 月统计页:「时段 / Hours」tab(拍完切回)
    "daily_top_app": (300, 553),  # 日统计页:最常用应用第一行(弹应用明细)⚠️ 待 --where 复核
    "chat_first":    (250, 145),  # 对话页:历史会话第一条 ⚠️ 待 --where 复核
}

# 每一步的镜头脚本:nav/click = 点 COORDS 里的键,shot = 截图存文件,esc = 关弹窗
SHOTS = [
    ("daily",       [("nav", "nav_daily"), ("shot", "daily.png")]),
    ("app_detail",  [("click", "daily_top_app"), ("shot", "app_detail.png"), ("esc",)]),
    ("weekly",      [("nav", "nav_weekly"), ("shot", "weekly.png")]),
    ("monthly",     [("nav", "nav_monthly"), ("shot", "monthly.png")]),
    ("monthly_cal", [("click", "tab_ratio"), ("shot", "monthly_cal.png"), ("click", "tab_hours")]),
    ("ai_summary",  [("nav", "nav_summary"), ("shot", "ai_summary.png")]),
    ("ai_chatbot",  [("nav", "nav_chat"), ("click", "chat_first"), ("shot", "ai_chatbot.png")]),
    ("cloud_sync",  [("nav", "nav_cloud"), ("shot", "cloud_sync.png")]),
]

# 遮蔽区(--no-mask 全部关闭),窗口相对坐标。
# 每条 = ((x0, y0, x1, y1), (采样点 x, y)):区域用采样点的背景色糊成圆角块。
# GLOBAL_MASKS 每张图都套;EXTRA_MASKS 按输出文件名追加。
GLOBAL_MASKS = [
    # 侧边栏账号区:只糊邮箱文字,保留左侧云朵图标(x<46)
    ((46, 85, 152, 117), (12, 101)),
]
EXTRA_MASKS = {
    "cloud_sync.png": [
        # Google Drive 卡片里的邮箱用户名(保留 @gmail.com 后缀)
        ((300, 138, 387, 159), (520, 148)),
    ],
}

SETTLE_NAV = 1.2    # 切页后等待渲染(秒)
SETTLE_CLICK = 0.8  # 页内点击(弹窗/tab)后的等待
SETTLE_SHOT = 0.3   # 按快门前的静置

# ───────────────────────── 系统交互 ─────────────────────────

def find_window():
    """找到演示实例主窗口:返回 {id, x, y, w, h},找不到返回 None。

    用 Quartz(pyobjc)读窗口表——pynput 的 macOS 后端依赖它,
    装 HumanMoveMouse 时会一并带上,不引入额外依赖。
    """
    try:
        import Quartz
    except ImportError:
        sys.exit("缺依赖:pip3 install HumanMoveMouse(会连带装上 pyobjc/Quartz)")
    opts = (
        Quartz.kCGWindowListOptionOnScreenOnly
        | Quartz.kCGWindowListExcludeDesktopElements
    )
    best = None
    for w in Quartz.CGWindowListCopyWindowInfo(opts, Quartz.kCGNullWindowID) or []:
        owner = (w.get(Quartz.kCGWindowOwnerName) or "").lower()
        if "hindsight" not in owner or w.get(Quartz.kCGWindowLayer, 1) != 0:
            continue
        b = w.get(Quartz.kCGWindowBounds) or {}
        wd, ht = b.get("Width", 0), b.get("Height", 0)
        if wd < 400 or ht < 300:  # 跳过菜单栏小窗
            continue
        if not best or wd * ht > best["w"] * best["h"]:
            best = {
                "id": int(w[Quartz.kCGWindowNumber]),
                "x": int(b["X"]), "y": int(b["Y"]),
                "w": int(wd), "h": int(ht),
            }
    return best


def screenshot(window_id, path: Path):
    path.parent.mkdir(parents=True, exist_ok=True)
    subprocess.run(
        ["screencapture", "-x", "-o", "-l", str(window_id), str(path)], check=True
    )


def apply_masks(path, win, filename):
    """遮蔽敏感区:按截图实际分辨率换算(Retina 2x),
    每个区域用各自采样点的背景色填圆角矩形,看不出补丁。"""
    from PIL import Image, ImageDraw

    regions = GLOBAL_MASKS + EXTRA_MASKS.get(filename, [])
    if not regions:
        return
    img = Image.open(path).convert("RGB")
    scale = img.width / win["w"]
    draw = ImageDraw.Draw(img)
    for (x0, y0, x1, y1), (sx, sy) in regions:
        box = tuple(round(v * scale) for v in (x0, y0, x1, y1))
        px = (
            min(img.width - 1, max(0, round(sx * scale))),
            min(img.height - 1, max(0, round(sy * scale))),
        )
        draw.rounded_rectangle(box, radius=round(6 * scale), fill=img.getpixel(px))
    img.save(path)


def activate_app():
    """把应用置前:macOS 对未聚焦窗口的首次点击只做激活、不传给页面,
    不先置前的话第一张图永远拍的是"上次停留的页面"。"""
    subprocess.run(
        ["osascript", "-e",
         'tell application "System Events" to '
         'set frontmost of (first process whose name contains "hindsight") to true'],
        check=True,
    )


def press_esc():
    subprocess.run(
        ["osascript", "-e", 'tell application "System Events" to key code 53'],
        check=True,
    )


# ───────────────────────── 模式 ─────────────────────────

def mode_where():
    """校准:悬停即打印窗口相对坐标,Ctrl-C 结束。"""
    try:
        from pynput.mouse import Controller
    except ImportError:
        sys.exit("需要 pynput(HumanMoveMouse 会带上):pip3 install HumanMoveMouse")
    mouse = Controller()
    win = find_window()
    if not win:
        sys.exit("没找到 Hindsight 窗口——先把演示实例跑起来。")
    print(f"窗口 @({win['x']},{win['y']}) {win['w']}×{win['h']};悬停读数,Ctrl-C 结束:")
    try:
        while True:
            x, y = mouse.position
            print(f"  窗口相对 ({int(x - win['x'])}, {int(y - win['y'])})      ", end="\r")
            time.sleep(0.5)
    except KeyboardInterrupt:
        print()


def mode_shoot(lang, only, speed, mask=True):
    try:
        from humanmouse import HumanMouseController
    except ImportError:
        sys.exit("缺依赖:pip3 install HumanMoveMouse")

    out_dir = LANG_DIRS[lang]
    win = find_window()
    if not win:
        sys.exit("没找到 Hindsight 窗口——先 npm run demo 起演示实例、切好语言再跑。")

    # 直线模式:UI 小目标要像素级落点;速度可调
    ctl = HumanMouseController(straight=True, speed_factor=speed)

    def click(key):
        rx, ry = COORDS[key]
        ctl.click_at((win["x"] + rx, win["y"] + ry))

    print(f"[shoot] 窗口 {win['w']}×{win['h']} → {out_dir.relative_to(REPO)}")
    activate_app()
    time.sleep(0.5)
    ctl.click_at((win["x"] + 500, win["y"] + 45))  # 页头无害区,吃掉激活性点击
    time.sleep(0.5)
    taken = []
    for name, steps in SHOTS:
        if only and name not in only:
            continue
        for step in steps:
            kind = step[0]
            if kind == "nav":
                click(step[1]); time.sleep(SETTLE_NAV)
            elif kind == "click":
                click(step[1]); time.sleep(SETTLE_CLICK)
            elif kind == "esc":
                press_esc(); time.sleep(SETTLE_CLICK)
            elif kind == "shot":
                time.sleep(SETTLE_SHOT)
                p = out_dir / step[1]
                screenshot(win["id"], p)
                if mask:
                    apply_masks(p, win, step[1])
                taken.append(p)
                print(f"  ✓ {step[1]}")
    print(f"[shoot] 完成:{len(taken)} 张 → {out_dir}")
    if not only or "cloud_sync" in (only or set()):
        print("[提醒] cloud_sync 是演示库的未登录态;要「已连接」效果请用真实实例手动截并打码邮箱。")


def main():
    ap = argparse.ArgumentParser(description="README 截图一键更新(先手动起演示实例+切语言)")
    ap.add_argument("--lang", choices=sorted(LANG_DIRS), help="目标语言目录")
    ap.add_argument("--only", help="只拍这些镜头,逗号分隔,如 daily,weekly")
    ap.add_argument("--speed", type=float, default=2.0, help="鼠标速度倍率(默认 2.0)")
    ap.add_argument("--no-mask", action="store_true", help="不遮蔽左上角账号邮箱")
    ap.add_argument("--where", action="store_true", help="坐标校准:悬停打印窗口相对坐标")
    args = ap.parse_args()

    if args.where:
        mode_where()
        return
    if not args.lang:
        ap.error("--lang 必填(或用 --where 校准坐标)")
    valid = {name for name, _ in SHOTS}
    only = None
    if args.only:
        only = set(args.only.split(","))
        unknown = only - valid
        if unknown:
            ap.error(f"未知镜头 {sorted(unknown)};可选:{sorted(valid)}")
    mode_shoot(args.lang, only, args.speed, mask=not args.no_mask)


if __name__ == "__main__":
    main()
