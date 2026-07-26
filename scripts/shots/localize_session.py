#!/usr/bin/env python3
"""把一条录好的巡场会话变换成五个语言版本——语言敏感控件按语言吸附对齐。

背景:界面语言切换后,带文字的头部控件(‹ / 今天胶囊 / 设备下拉 / 占比 tab)
会因文字宽度不同而横向漂移(pt 的 Proporção 比中文「占比」漂 72px),
按 zh 录的轨迹在别的语言下就点空。侧边栏图标行、内容行不漂移,原样保留。

用法:
    uv run scripts/shots/localize_session.py            # 默认吃 tour_session.jsonl
    uv run scripts/shots/localize_session.py --file take2.jsonl

产物:tour_session_{zh,tw,en,ja,pt}.jsonl + 同名 .win.json(锚点原样复制)。
回放:切好界面语言后
    uv run scripts/shots/rec.py play --file scripts/shots/tour_session_en.jsonl

变换规则:
  - 语言敏感点击吸附到该语言控件中心 x(y 保留)——比保留"点边缘"的
    原始习惯更抗差;zh 版也做吸附,原轨迹里贴边的点击一并治好;
  - 设备下拉菜单项跟随下拉框的 Δx(菜单锚定在下拉框上,行高不随语言变);
  - 每个被移动的点击,前后 RAMP 秒内的移动轨迹线性扭入/扭出,回放是平滑弧线。

坐标基准:960×720 窗口,控件中心从五语言 daily.png 实测。改窗口尺寸要重量。
"""

# /// script
# requires-python = ">=3.10"
# ///

import argparse
import json
import shutil
from pathlib import Path

BASE_LANG = "zh"  # 原始录制的界面语言
RAMP = 0.45       # 点击前/后轨迹扭曲窗口(秒)

# 柱状图内容点击的"钳位":按下事件序号(0 起,只数 pressed)。
# 柱高随数据变,y 钳到柱根(两图基线像素实测都在 ≈381-383,钳 372);
# 月图柱窄(13px)缝宽(8px),x 还要吸附到柱心——494 = 7 月 12 号柱心(像素扫描实测)。
# ⚠️ 重录轨迹后按脚本尾部打印的按下点序号表重标这两张表。
BAR_CLICK_Y = {0: 372, 1: 372, 11: 372, 12: 372}
BAR_CLICK_X = {11: 494, 12: 494}


# 语言敏感控件的中心 x(窗口相对,y 都在 157 行;从各语言 daily.png 实测)
CTRL_X = {
    "tab_ratio": {"zh": 305, "tw": 306, "en": 330, "ja": 300, "pt": 377},
    "device_dd": {"zh": 660, "tw": 660, "en": 665, "ja": 660, "pt": 665},
    "btn_prev":  {"zh": 790, "tw": 790, "en": 795, "ja": 790, "pt": 795},
    "pill":      {"zh": 841, "tw": 841, "en": 847, "ja": 841, "pt": 846},
}
# 命中框(窗口相对):落进框内的点击视为点了该控件
CTRL_BOX = {
    "tab_ratio": (280, 143, 335, 171),
    "device_dd": (555, 143, 767, 171),
    "btn_prev":  (775, 143, 805, 171),
    "pill":      (816, 143, 868, 171),
}
# 设备下拉展开后的菜单区(x 跟随下拉框平移,y 不动)
MENU_BOX = (555, 175, 767, 285)


def classify(rx, ry):
    for name, (x0, y0, x1, y1) in CTRL_BOX.items():
        if x0 <= rx <= x1 and y0 <= ry <= y1:
            return name
    x0, y0, x1, y1 = MENU_BOX
    if x0 <= rx <= x1 and y0 <= ry <= y1:
        return "menu"
    return None


def localize(events, anchor, lang):
    ax, ay = anchor["x"], anchor["y"]
    out = [dict(e) for e in events]

    # 第一遍:给每个"按下"算位移(dx, dy);"抬起"沿用它前面那次按下的位移
    def press_delta(idx, rx, ry):
        if idx in BAR_CLICK_Y:
            dx = float(BAR_CLICK_X[idx] - rx) if idx in BAR_CLICK_X else 0.0
            return dx, float(BAR_CLICK_Y[idx] - ry)
        kind = classify(rx, ry)
        if kind == "menu":
            return float(CTRL_X["device_dd"][lang] - CTRL_X["device_dd"][BASE_LANG]), 0.0
        if kind:
            return float(CTRL_X[kind][lang] - rx), 0.0
        return 0.0, 0.0

    shifts = []          # (t_press, dx, dy) 用于轨迹扭曲
    cur = (0.0, 0.0)     # 抬起沿用的当前位移
    press_idx = -1
    deltas = []          # 与 out 等长,click 事件的 (dx, dy)
    for e in out:
        if e["type"] == "click":
            if e.get("pressed"):
                press_idx += 1
                rx, ry = e["x"] - ax, e["y"] - ay
                cur = press_delta(press_idx, rx, ry)
                if cur != (0.0, 0.0):
                    shifts.append((e["t"], cur[0], cur[1]))
            deltas.append(cur)
        else:
            deltas.append(None)

    if not shifts:
        return out

    def delta_at(t):
        dx = dy = 0.0
        for tc, sx, sy in shifts:
            if tc - RAMP <= t <= tc:
                w = (t - (tc - RAMP)) / RAMP      # 扭入
            elif tc < t <= tc + RAMP:
                w = 1 - (t - tc) / RAMP           # 扭出
            else:
                continue
            dx += sx * w
            dy += sy * w
        return dx, dy

    for e, d in zip(out, deltas):
        if e["type"] == "move":
            mx, my = delta_at(e["t"])
            e["x"] = round(e["x"] + mx)
            e["y"] = round(e["y"] + my)
        elif e["type"] == "click" and d:
            e["x"] = round(e["x"] + d[0])
            e["y"] = round(e["y"] + d[1])
    return out


def main():
    ap = argparse.ArgumentParser(description="巡场会话按语言对齐,生成五份")
    ap.add_argument("--file", type=Path,
                    default=Path(__file__).with_name("tour_session.jsonl"))
    args = ap.parse_args()
    session, anchor_file = args.file, args.file.with_suffix(".win.json")
    if not session.is_file() or not anchor_file.is_file():
        raise SystemExit(f"缺 {session} 或 {anchor_file}")
    events = [json.loads(l) for l in open(session)]
    anchor = json.loads(anchor_file.read_text())

    for lang in ("zh", "tw", "en", "ja", "pt"):
        out_events = localize(events, anchor, lang)
        out = session.with_name(f"{session.stem}_{lang}.jsonl")
        with open(out, "w") as g:
            for e in out_events:
                g.write(json.dumps(e) + "\n")
        shutil.copy(anchor_file, out.with_suffix(".win.json"))
        moved = sum(
            1 for a, b in zip(events, out_events)
            if a["type"] == "click" and a.get("pressed")
            and (a["x"], a["y"]) != (b["x"], b["y"])
        )
        print(f"✓ {out.name}(移动了 {moved} 个按下点)")

    ax, ay = anchor["x"], anchor["y"]
    print("── 按下点序号对照(重录后据此重标 BAR_CLICKS):")
    i = -1
    for e in events:
        if e["type"] == "click" and e.get("pressed"):
            i += 1
            print(f"  #{i:2d}  t={e['t']:5.1f}  rel=({e['x']-ax:4},{e['y']-ay:4})")


if __name__ == "__main__":
    main()
