#!/usr/bin/env python3
"""核心功能演示巡场(macOS)——给 GitHub README 录无声小视频用的鼠标走位。

40 秒级、无解说、适合静音循环播放:节奏偏快,每页只停到"看清了"为止。
巡场路线:
    日统计(今天 → 昨天 → 切回 → 切设备)
    → 月统计 → 月度占比
    → 对话(点开准备好的那条结果,慢滚)
    → AI 总结(滚到有内容的段落)
    → 云同步 → 回日统计定格收尾

用法:
    npm run demo                       # 起演示实例,切好语言,准备好对话结果
    uv run scripts/shots/tour.py --plan    # 只打印时间轴,不动鼠标
    uv run scripts/shots/tour.py           # 正式巡场(先给 3 秒切到录屏)

坐标:复用 shoot.py 的 COORDS(窗口相对,960×720 基准),本文件只补
巡场特有的几个控件。换窗口尺寸后用 `uv run scripts/shots/shoot.py --where`
悬停读数重校。

节奏:--speed 调鼠标移动速度(默认 1.1,无声视频节奏偏快);停留时长在
STEPS 里按步微调。跑之前把系统鼠标指针调大一号,录出来更清楚。
"""

# /// script
# requires-python = ">=3.10"
# dependencies = ["HumanMoveMouse"]
# ///

import argparse
import os
import sys
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from shoot import COORDS, activate_app, find_window  # noqa: E402

# ───────────────────────── 巡场专用坐标(窗口相对,960×720)─────────────────────────
# 日/月统计页头部同一行(y≈157):时段/占比 tab、设备下拉、‹ 今天/本月 ›
TOUR_COORDS = {
    "btn_prev":     (773, 157),  # ‹ 上一天 / 上一月
    "btn_next":     (899, 157),  # › 下一天
    "device_dd":    (643, 157),  # 设备下拉(所有设备)
    "device_item1": (643, 192),  # 下拉展开后第 1 项(所有设备)⚠️ 待校准
    "device_item2": (643, 226),  # 下拉展开后第 2 项(本机·演示)⚠️ 待校准
    "park":         (500, 690),  # 停留时鼠标的"待机位",避免悬停高亮挡内容
    "content_mid":  (560, 400),  # 滚动前把鼠标放到内容区中央
}


def C(key):
    return TOUR_COORDS[key] if key in TOUR_COORDS else COORDS[key]


# ───────────────────────── 巡场脚本 ─────────────────────────
# 每步 = (动作, 参数, 停留秒, 解说)。动作:
#   click  = 仿人轨迹移动并点击 → 鼠标退回待机位 → 停留
#   hover  = 只移动不点击(用于把光标从画面焦点挪开)
#   scroll = 在内容区平滑滚动 N 格(负数向下)
STEPS = [
    ("click", "nav_daily",    2.6, "日统计·今天全景"),
    ("click", "btn_prev",     2.0, "切到昨天"),
    ("click", "btn_next",     1.2, "切回今天"),
    ("click", "device_dd",    0.8, "打开设备下拉"),
    ("click", "device_item2", 2.0, "只看本机(演示)"),
    ("click", "device_dd",    0.6, "再开下拉"),
    ("click", "device_item1", 1.0, "切回所有设备"),
    ("click", "nav_monthly",  2.6, "月统计"),
    ("click", "tab_ratio",    3.0, "月度占比:环形图 + 环比"),
    ("click", "nav_chat",     0.9, "进对话页"),
    ("click", "chat_first",   3.5, "点开准备好的问答"),
    ("scroll", -4,            2.0, "慢滚看完回答"),
    ("click", "nav_summary",  1.0, "AI 总结·日报"),
    ("scroll", -5,            3.0, "滚到有内容的段落"),
    ("click", "nav_cloud",    2.8, "云同步:开关 + 设备列表"),
    ("click", "nav_daily",    1.5, "回日统计定格收尾"),
]

SCROLL_STEP_PAUSE = 0.06  # 每格滚动间隔,越大滚得越慢


def total_seconds(speed):
    move = 0.9 / max(speed, 0.1)  # 每次仿人移动的粗略耗时
    t = sum(d for _, _, d, _ in STEPS) + sum(
        move for a, *_ in STEPS if a in ("click", "hover")
    )
    return round(t)


def run(speed):
    try:
        from humanmouse import HumanMouseController
        from pynput.mouse import Controller as PynputMouse
    except ImportError:
        sys.exit("缺依赖:pip3 install HumanMoveMouse(或直接用 uv run)")

    win = find_window()
    if not win:
        sys.exit("没找到 Hindsight 窗口——先 npm run demo 起演示实例。")

    ctl = HumanMouseController(speed_factor=speed)  # 仿人轨迹,不走直线模式
    wheel = PynputMouse()

    def to_abs(rel):
        return (win["x"] + rel[0], win["y"] + rel[1])

    print(f"[tour] 窗口 {win['w']}×{win['h']},预计全程约 {total_seconds(speed)} 秒。")
    print("[tour] 3 秒后开始——切到录屏,别碰鼠标键盘。")
    time.sleep(3)
    activate_app()
    time.sleep(0.4)
    ctl.click_at(to_abs((500, 45)))  # 页头无害区,吃掉激活性点击
    time.sleep(0.6)

    for step in STEPS:
        action, arg, dwell, note = step
        print(f"  ▸ {note}")
        if action == "click":
            ctl.click_at(to_abs(C(arg)))
            time.sleep(0.35)
            ctl.move_to(to_abs(TOUR_COORDS["park"]))  # 光标退开,不挡内容
        elif action == "hover":
            ctl.move_to(to_abs(C(arg)))
        elif action == "scroll":
            ctl.move_to(to_abs(TOUR_COORDS["content_mid"]))
            time.sleep(0.2)
            n = int(arg)
            for _ in range(abs(n)):
                wheel.scroll(0, -1 if n < 0 else 1)
                time.sleep(SCROLL_STEP_PAUSE)
            ctl.move_to(to_abs(TOUR_COORDS["park"]))
        time.sleep(dwell)
    print("[tour] 巡场结束。")


def main():
    ap = argparse.ArgumentParser(description="核心功能演示巡场(录屏用)")
    ap.add_argument("--speed", type=float, default=1.1,
                    help="鼠标移动速度倍率(默认 1.1,无声视频节奏偏快;越小越慢)")
    ap.add_argument("--plan", action="store_true", help="只打印时间轴,不动鼠标")
    args = ap.parse_args()

    if args.plan:
        t = 3.0
        move = 0.9 / max(args.speed, 0.1)
        print(f"{'时刻':>6}  {'停留':>4}  解说")
        for action, arg, dwell, note in STEPS:
            cost = (move if action in ("click", "hover") else 0.6) + dwell
            print(f"{t:6.1f}s  {dwell:3.1f}s  {note}")
            t += cost
        print(f"── 预计全程 ≈ {round(t)} 秒(不含你自己的片头片尾)")
        return
    run(args.speed)


if __name__ == "__main__":
    main()
