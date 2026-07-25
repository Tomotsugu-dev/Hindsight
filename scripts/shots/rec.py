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
from shoot import find_window  # noqa: E402

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
        if any(win[k] != frame[k] for k in ("x", "y", "w", "h")):
            print(f"[rec] 窗口已复位到录制时的框架 {frame['x']},{frame['y']} "
                  f"{frame['w']}×{frame['h']}")
            restore_window(frame)
            time.sleep(0.5)
    else:
        print("[rec] ⚠️ 没有窗口锚点文件,按绝对坐标原样回放——确保窗口没挪过。")
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
