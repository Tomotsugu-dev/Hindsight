#!/usr/bin/env python3
"""录你自己的演示操作,之后精确回放——录屏素材就用回放那一遍,每条完全一致。

    uv run scripts/shots/rec.py record            # 3 秒倒计时后开录;按 F10 停止并保存
    uv run scripts/shots/rec.py play              # 3 秒倒计时后原速回放;Esc 随时中止
    uv run scripts/shots/rec.py play --speed 1.2  # 加速回放(剪辑觉得拖沓时)
    uv run scripts/shots/rec.py record --file take2.jsonl   # 多条 take 各存各的

说明:
  - 只录鼠标:移动的完整平滑轨迹 + 点击,逐事件原时序回放。
  - 不录键盘——需要打字的镜头(比如对话提问)提前把结果准备好,纯鼠标巡场。
  - 会话文件默认 scripts/shots/tour_session.jsonl,仅本地(已 gitignore)。
  - 回放务必走本脚本(rec.py play),别直接 `humanmouse play`——原生 CLI
    没有窗口复位,窗口挪过 20px 就整场点空。
  - 底层录的是绝对屏幕坐标,但录制时会同时存一份窗口锚点(位置+尺寸,
    .win.json),回放前自动把应用窗口挪回录制时的框架——窗口挪过、变过位
    都没关系;改过"尺寸"也会被一并复原。
"""

# /// script
# requires-python = ">=3.10"
# dependencies = ["HumanMoveMouse"]
# ///

import argparse
import json
import os
import subprocess
import sys
import time
from pathlib import Path

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from shoot import activate_app, find_window  # noqa: E402

DEFAULT_SESSION = Path(__file__).with_name("tour_session.jsonl")


def anchor_path(session: Path) -> Path:
    return session.with_suffix(".win.json")


def restore_window(frame):
    """把应用窗口挪回录制时的框架(位置+尺寸)。"""
    subprocess.run(
        ["osascript",
         "-e", 'tell application "System Events" to tell '
               '(first process whose name contains "hindsight")',
         "-e", f'set position of front window to {{{frame["x"]}, {frame["y"]}}}',
         "-e", f'set size of front window to {{{frame["w"]}, {frame["h"]}}}',
         "-e", "end tell"],
        check=True,
    )


def countdown(n, msg):
    print(msg)
    for i in range(n, 0, -1):
        print(f"  {i}…", flush=True)
        time.sleep(1)


def do_record(path: Path):
    from humanmouse import Recorder

    win = find_window()
    if not win:
        sys.exit("[rec] 没找到 Hindsight 窗口——先把应用跑起来。")
    countdown(3, "[rec] 3 秒后开始录制——切到应用窗口。按 F10 停止。")
    rec = Recorder(capture_keyboard=False, stop_hotkey="f10")
    rec.start()
    print("[rec] 录制中…(F10 结束)")
    rec.wait()
    path.parent.mkdir(parents=True, exist_ok=True)
    rec.save(str(path))
    anchor_path(path).write_text(json.dumps({k: win[k] for k in ("x", "y", "w", "h")}))
    print(f"[rec] 已保存 → {path}(窗口锚点 {win['x']},{win['y']} {win['w']}×{win['h']})")


def shifted_session(path: Path, dx: int, dy: int) -> Path:
    """复位钉不住时的兜底:把整条会话的事件坐标平移 (dx, dy) 后另存回放。"""
    out = path.with_name(path.stem + ".shifted.jsonl")
    with open(path) as f, open(out, "w") as g:
        for line in f:
            ev = json.loads(line)
            if "x" in ev:
                ev["x"] += dx
                ev["y"] += dy
            g.write(json.dumps(ev) + "\n")
    return out


def normalize_tabs(win):
    """回放前的状态归位:日/月统计的「时段/占比」页签有记忆,上一次回放
    切到占比后就停在那里,下一次回放的柱状图点击会全部落空。
    把两页强制拨回「时段」,再回到日统计当起点。
    坐标是五种语言 Hours tab 的交集区(x=245 全命中),语言无关。"""
    from humanmouse import HumanMouseController

    ctl = HumanMouseController(straight=True, speed_factor=3.0)
    activate_app()
    time.sleep(0.4)

    def click(rx, ry, wait=0.9):
        ctl.click_at((win["x"] + rx, win["y"] + ry))
        time.sleep(wait)

    click(500, 45, 0.4)   # 唤醒点击(未聚焦窗口首击会被吞)
    click(70, 165)        # 日统计
    click(245, 157, 0.5)  # 时段 tab
    click(70, 241)        # 月统计
    click(245, 157, 0.5)  # 时段 tab
    click(70, 165)        # 回日统计(录制起点)


def do_play(path: Path, speed: float, loop: int):
    from humanmouse import play_file

    if not path.is_file():
        sys.exit(f"[rec] 找不到会话文件:{path}(先 record 一条)")
    ap_ = anchor_path(path)
    if ap_.is_file():
        frame = json.loads(ap_.read_text())
        win = find_window()
        if not win:
            sys.exit("[rec] 没找到 Hindsight 窗口——先把应用跑起来。")
        if win["w"] != frame["w"] or win["h"] != frame["h"]:
            sys.exit(f"[rec] 窗口尺寸和录制时不同({win['w']}×{win['h']} vs "
                     f"{frame['w']}×{frame['h']}),坐标没法对齐——调回原尺寸或重录。")
        if win["x"] != frame["x"] or win["y"] != frame["y"]:
            print(f"[rec] 窗口复位到录制时的位置 {frame['x']},{frame['y']}")
            restore_window(frame)
            time.sleep(0.5)
            now = find_window()
            if now["x"] != frame["x"] or now["y"] != frame["y"]:
                dx, dy = now["x"] - frame["x"], now["y"] - frame["y"]
                print(f"[rec] 复位没钉住(仍差 {dx},{dy}),改用事件平移兜底。")
                path = shifted_session(path, dx, dy)
    else:
        print("[rec] ⚠️ 没有窗口锚点文件,按绝对坐标原样回放——确保窗口没挪过。")
    win = find_window()
    if win:
        print("[rec] 状态归位:日/月统计页签拨回「时段」,回到日统计起点。")
        normalize_tabs(win)
    countdown(3, f"[rec] 3 秒后开始回放({speed}x ×{loop} 遍)——开录屏,Esc 可中止。")
    play_file(str(path), speed=speed, loop=loop)
    print("[rec] 回放结束。")


def main():
    ap = argparse.ArgumentParser(description="演示操作录制 / 精确回放(录屏素材用)")
    sub = ap.add_subparsers(dest="cmd", required=True)
    p_rec = sub.add_parser("record", help="录一条(F10 停止)")
    p_rec.add_argument("--file", type=Path, default=DEFAULT_SESSION)
    p_play = sub.add_parser("play", help="回放(Esc 中止)")
    p_play.add_argument("--file", type=Path, default=DEFAULT_SESSION)
    p_play.add_argument("--speed", type=float, default=1.0)
    p_play.add_argument("--loop", type=int, default=1)
    args = ap.parse_args()

    if args.cmd == "record":
        do_record(args.file)
    else:
        do_play(args.file, args.speed, args.loop)


if __name__ == "__main__":
    main()
