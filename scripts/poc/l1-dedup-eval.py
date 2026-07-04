#!/usr/bin/env python3
"""L1 静止门离线评测:拿真实截图流跑设计定案的算法,统计附着率并生成可视化报告。

用法:
    python l1-dedup-eval.py <截图日期目录> <sqlite路径> [输出html]

算法与 docs/design/screen-memory.md §3/§10 定案一致:
    256×144 灰度缩略图,逐像素 |Δ|>τ(12) 记变化,占比 < f(0.10%) 附着;
    越阈帧挂起 1 帧做持久性确认(瞬态弹层拦截);
    事件帧豁免:挂起帧与锚点窗口标题不同 → 免确认直接落盘;
    标题守卫:两帧标题都非空且不同 → 禁止附着(像素再像也不附);
    K=4 阀:连续附着 4 帧后第 5 帧无条件落盘并更新锚点。

输出为本地 HTML(截图缩略图内嵌 base64,不出设备),勿提交 git。
"""

import base64
import io
import os
import sqlite3
import sys
from PIL import Image, ImageChops

TAU = 12          # 单像素灰度差阈值
F = 0.001         # 变化占比阈值 0.10%
K = 4             # 连续附着 K 帧后强制落盘
THUMB_W, THUMB_H = 256, 144
TOTAL_PX = THUMB_W * THUMB_H

RESAMPLE = getattr(Image, "Resampling", Image).BILINEAR


class Frame:
    def __init__(self, path, app, title):
        self.path = path
        self.name = os.path.basename(path)
        self.size = os.path.getsize(path)
        # 文件名 HHMMSS_mmm.jpg → 当日秒数
        hms = self.name.split("_")[0]
        self.secs = int(hms[0:2]) * 3600 + int(hms[2:4]) * 60 + int(hms[4:6])
        self.ts = f"{hms[0:2]}:{hms[2:4]}:{hms[4:6]}"
        self.app = app or "?"
        self.title = title or ""
        self.thumb = None      # 256×144 灰度
        self.decision = None   # kept / attached / transient
        self.reason = ""       # first / confirmed / event / title-guard / k-valve / eos
        self.frac = None       # 判定时相对锚点的变化占比
        self.rep = None        # 附着目标(锚点帧)


def load_thumb(fr):
    if fr.thumb is None:
        with Image.open(fr.path) as im:
            fr.thumb = im.convert("L").resize((THUMB_W, THUMB_H), RESAMPLE)
    return fr.thumb


def frac(a, b):
    """两张缩略图中 |Δ|>τ 的像素占比。"""
    hist = ImageChops.difference(load_thumb(a), load_thumb(b)).histogram()
    return sum(hist[TAU + 1:]) / TOTAL_PX


def titles_block(a, b):
    """标题守卫:两帧标题都非空且不同 → 禁止附着。"""
    return bool(a.title) and bool(b.title) and a.title != b.title


def run_gate(frames):
    """跑 L1 静止门,填充每帧的 decision/reason/frac/rep。"""
    anchor = None
    pending = None
    consec = 0

    def keep(fr, reason):
        nonlocal anchor, consec
        fr.decision, fr.reason = "kept", reason
        anchor, consec = fr, 0

    def attach(fr, reason=""):
        nonlocal consec
        fr.decision, fr.reason, fr.rep = "attached", reason, anchor
        consec += 1

    for fr in frames:
        if anchor is None:
            fr.frac = 1.0
            keep(fr, "first")
            continue

        fr.frac = frac(fr, anchor)
        guard = titles_block(fr, anchor)

        if pending is not None:
            p = pending
            pending = None
            if fr.frac < F and not guard:
                # 证人显示变化已消失 → 挂起帧是瞬态弹层,本帧照常附着
                p.decision, p.reason, p.rep = "transient", "reverted", anchor
                attach(fr)
                continue
            # 持久变化确认 → 挂起帧落盘,锚点 := 挂起帧,本帧重新评估
            keep(p, "confirmed")
            fr.frac = frac(fr, anchor)
            guard = titles_block(fr, anchor)

        if consec >= K:
            keep(fr, "k-valve")   # 兜底阀:无条件落盘并更新锚点
            continue

        if fr.frac < F:
            if guard:
                keep(fr, "title-guard")   # 像素几乎没变但标题变了,禁止附着
            else:
                attach(fr)
            continue

        if guard:
            keep(fr, "event")   # 事件帧(标题已变):免持久性确认直接落盘
        else:
            pending = fr        # 同标题越阈 → 挂起等下一帧确认

    if pending is not None:
        keep(pending, "eos")    # 流结束,挂起帧保守落盘


# ---------- 可视化 ----------

CSS = """
:root {
  --surface-1: #fcfcfb; --page: #f9f9f7;
  --ink-1: #0b0b0b; --ink-2: #52514e; --ink-3: #898781;
  --grid: #e1e0d9; --axis: #c3c2b7; --border: rgba(11,11,11,0.10);
  --kept: #2a78d6; --attached: #1baf7a; --transient: #eda100;
}
@media (prefers-color-scheme: dark) {
  :root {
    --surface-1: #1a1a19; --page: #0d0d0d;
    --ink-1: #ffffff; --ink-2: #c3c2b7; --ink-3: #898781;
    --grid: #2c2c2a; --axis: #383835; --border: rgba(255,255,255,0.10);
    --kept: #3987e5; --attached: #199e70; --transient: #c98500;
  }
}
* { box-sizing: border-box; margin: 0; }
body { background: var(--page); color: var(--ink-1);
       font: 14px/1.6 system-ui, -apple-system, "Segoe UI", sans-serif;
       padding: 24px; }
.wrap { max-width: 1080px; margin: 0 auto; }
h1 { font-size: 20px; margin-bottom: 4px; }
h2 { font-size: 15px; margin: 28px 0 10px; }
.sub { color: var(--ink-2); font-size: 13px; margin-bottom: 20px; }
.card { background: var(--surface-1); border: 1px solid var(--border);
        border-radius: 10px; padding: 16px; margin-bottom: 16px; }
.tiles { display: grid; grid-template-columns: repeat(5, 1fr); gap: 12px; }
.tile .v { font-size: 26px; font-weight: 650; }
.tile .l { font-size: 12px; color: var(--ink-3); }
.legend { display: flex; gap: 16px; font-size: 12px; color: var(--ink-2);
          margin-bottom: 8px; flex-wrap: wrap; }
.legend span { display: inline-flex; align-items: center; gap: 6px; }
.sw { width: 10px; height: 10px; border-radius: 3px; display: inline-block; }
svg { display: block; max-width: 100%; }
svg text { font: 11px system-ui, sans-serif; fill: var(--ink-3); }
h3 { font-size: 13px; color: var(--ink-2); margin: 16px 0 8px; }
.grid { display: grid; grid-template-columns: repeat(auto-fill, minmax(176px, 1fr));
        gap: 10px; }
.fcard { border: 2px solid var(--border); border-radius: 8px; padding: 4px;
         background: var(--surface-1); }
.fcard img { width: 100%; border-radius: 5px; display: block; }
.fcard .cap { font-size: 11px; color: var(--ink-2); line-height: 1.45;
              margin-top: 3px; }
.note { color: var(--ink-2); font-size: 13px; }
"""


def b64_thumb(fr, width=340):
    with Image.open(fr.path) as im:
        h = max(1, round(im.height * width / im.width))
        small = im.convert("RGB").resize((width, h), RESAMPLE)
    buf = io.BytesIO()
    small.save(buf, "JPEG", quality=55)
    return "data:image/jpeg;base64," + base64.b64encode(buf.getvalue()).decode()


def svg_timeline(frames):
    """Timeline strip: one tick per frame, colored by decision (English labels)."""
    w, h, pad = 1040, 96, 34
    lo = min(f.secs for f in frames)
    hi = max(f.secs for f in frames)
    lo, hi = (lo // 3600) * 3600, ((hi // 3600) + 1) * 3600
    span = max(1, hi - lo)
    x = lambda s: pad + (s - lo) / span * (w - 2 * pad)
    parts = [f'<svg viewBox="0 0 {w} {h}" role="img" aria-label="Frame decisions over the day">']
    parts.append(f'<line x1="{pad}" y1="{h-26}" x2="{w-pad}" y2="{h-26}" stroke="var(--axis)"/>')
    for hr in range(lo, hi + 1, 3600):
        parts.append(f'<line x1="{x(hr):.1f}" y1="{h-26}" x2="{x(hr):.1f}" y2="{h-22}" stroke="var(--axis)"/>')
        parts.append(f'<text x="{x(hr):.1f}" y="{h-8}" text-anchor="middle">{hr//3600:02d}:00</text>')
    for fr in frames:
        color = {"kept": "var(--kept)", "attached": "var(--attached)",
                 "transient": "var(--transient)"}[fr.decision]
        y0 = 14 if fr.decision == "kept" else 34
        tip = f"{fr.ts} · {fr.app} · {fr.decision} ({fr.reason}) · diff {fr.frac*100:.3f}%"
        parts.append(
            f'<rect x="{x(fr.secs)-1:.1f}" y="{y0}" width="2.4" height="18" rx="1" fill="{color}">'
            f"<title>{esc(tip)}</title></rect>"
        )
    parts.append(f'<text x="{pad}" y="10">kept (top row) / attached &amp; transient (bottom row)</text>')
    parts.append("</svg>")
    return "".join(parts)


def svg_hist(frames):
    """Histogram of per-frame diff fraction vs anchor (log bins, English labels)."""
    import math
    w, h, pad_l, pad_b = 1040, 190, 44, 30
    edges = [0] + [10 ** (e / 2) for e in range(-8, 1)]  # 0, 1e-4 .. 100%
    labels = ["0", ".01%", ".03%", ".1%", ".3%", "1%", "3%", "10%", "32%", "100%"]
    bins = [0] * (len(edges))
    for fr in frames:
        v = fr.frac
        idx = 0
        for i in range(1, len(edges)):
            if v >= edges[i]:
                idx = i
        bins[idx] += 1
    top = max(bins) or 1
    bw = (w - pad_l - 20) / len(bins)
    parts = [f'<svg viewBox="0 0 {w} {h}" role="img" aria-label="Diff fraction distribution">']
    for g in range(0, top + 1, max(1, top // 4)):
        y = h - pad_b - g / top * (h - pad_b - 24)
        parts.append(f'<line x1="{pad_l}" y1="{y:.1f}" x2="{w-20}" y2="{y:.1f}" stroke="var(--grid)"/>')
        parts.append(f'<text x="{pad_l-6}" y="{y+4:.1f}" text-anchor="end">{g}</text>')
    for i, n in enumerate(bins):
        if n == 0:
            continue
        bh = n / top * (h - pad_b - 24)
        bx = pad_l + i * bw + 3
        parts.append(
            f'<rect x="{bx:.1f}" y="{h-pad_b-bh:.1f}" width="{bw-6:.1f}" height="{bh:.1f}" '
            f'rx="4" fill="var(--kept)"><title>{labels[i]} bin: {n} frames</title></rect>'
        )
        parts.append(f'<text x="{bx+(bw-6)/2:.1f}" y="{h-pad_b-bh-5:.1f}" text-anchor="middle" fill="var(--ink-2)">{n}</text>')
    for i, lab in enumerate(labels):
        parts.append(f'<text x="{pad_l+i*bw+(bw-6)/2:.1f}" y="{h-12}" text-anchor="middle">{lab}</text>')
    # 阈值参考线:f=0.10% 落在 ".1%" bin 的左边缘
    gate_x = pad_l + 3 * bw
    parts.append(f'<line x1="{gate_x:.1f}" y1="18" x2="{gate_x:.1f}" y2="{h-pad_b}" stroke="var(--ink-3)" stroke-dasharray="4 3"/>')
    parts.append(f'<text x="{gate_x+5:.1f}" y="16">attach gate f = 0.10% (left of line = attach candidate)</text>')
    parts.append("</svg>")
    return "".join(parts)


def svg_apps(frames):
    """Per-app stacked bars: kept vs attached (English labels)."""
    apps = {}
    for fr in frames:
        d = apps.setdefault(fr.app, {"kept": 0, "attached": 0})
        d["attached" if fr.decision in ("attached", "transient") else "kept"] += 1
    rows = sorted(apps.items(), key=lambda kv: -(kv[1]["kept"] + kv[1]["attached"]))[:8]
    w, rh, pad_l = 1040, 26, 210
    h = len(rows) * rh + 30
    top = max(v["kept"] + v["attached"] for _, v in rows)
    sx = (w - pad_l - 120) / top
    parts = [f'<svg viewBox="0 0 {w} {h}" role="img" aria-label="Frames and attach share per app">']
    for i, (app, v) in enumerate(rows):
        y = 10 + i * rh
        kw, aw = v["kept"] * sx, v["attached"] * sx
        name = app if len(app) <= 28 else app[:27] + "…"
        parts.append(f'<text x="{pad_l-8}" y="{y+13}" text-anchor="end" fill="var(--ink-2)">{esc(name)}</text>')
        parts.append(f'<rect x="{pad_l}" y="{y}" width="{max(kw,1):.1f}" height="18" rx="4" fill="var(--kept)"><title>{esc(app)}: kept {v["kept"]}</title></rect>')
        parts.append(f'<rect x="{pad_l+kw+2:.1f}" y="{y}" width="{max(aw,1):.1f}" height="18" rx="4" fill="var(--attached)"><title>{esc(app)}: attached {v["attached"]}</title></rect>')
        total = v["kept"] + v["attached"]
        rate = v["attached"] / total * 100 if total else 0
        parts.append(f'<text x="{pad_l+kw+aw+8:.1f}" y="{y+13}" fill="var(--ink-2)">{total} · {rate:.0f}% attached</text>')
    parts.append("</svg>")
    return "".join(parts)


def esc(s):
    return s.replace("&", "&amp;").replace("<", "&lt;").replace(">", "&gt;").replace('"', "&quot;")


def build_html(frames, out_path, date_str):
    total = len(frames)
    kept = [f for f in frames if f.decision == "kept"]
    attached = [f for f in frames if f.decision == "attached"]
    transient = [f for f in frames if f.decision == "transient"]
    saved = sum(f.size for f in attached + transient)
    total_bytes = sum(f.size for f in frames)
    rate = (len(attached) + len(transient)) / total * 100
    guard_kept = [f for f in kept if f.reason == "title-guard"]
    kvalve = [f for f in kept if f.reason == "k-valve"]

    # 全帧画廊:按小时分组,每帧缩略图 + 判定标签
    hours = {}
    for f in frames:
        hours.setdefault(f.secs // 3600, []).append(f)

    BADGE = {
        "kept": ("var(--kept)", "保留"),
        "attached": ("var(--attached)", "附着·免写"),
        "transient": ("var(--transient)", "瞬态·免写"),
    }

    def card(f):
        color, label = BADGE[f.decision]
        extra = f" → REP {f.rep.ts}" if f.rep is not None else ""
        reason = f" ({f.reason})" if f.decision == "kept" and f.reason != "confirmed" else ""
        return (
            f'<div class="fcard" style="border-color:{color}">'
            f'<img src="{b64_thumb(f, 260)}">'
            f'<div class="cap"><span class="sw" style="background:{color}"></span> <b>{label}</b>{esc(extra)}{reason}<br>'
            f"{f.ts} · diff {f.frac*100:.2f}% · {esc(f.app[:24])}</div></div>"
        )

    gallery = []
    for hr in sorted(hours):
        gallery.append(f"<h3>{hr:02d}:00 — {hr:02d}:59 · {len(hours[hr])} frames</h3>")
        gallery.append('<div class="grid">' + "".join(card(f) for f in hours[hr]) + "</div>")
    gallery_html = "".join(gallery)

    html = f"""<!doctype html><html lang="zh"><head><meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>L1 dedup eval {date_str}</title><style>{CSS}</style></head><body><div class="wrap">
<h1>L1 静止门离线评测 — {date_str}</h1>
<p class="sub">算法与 screen-memory.md 定案一致(τ=12 / f=0.10% / 持久性确认 / 事件帧豁免 / 标题守卫 / K={K} 阀)。
本页含真实截图缩略图,仅限本机查看,勿提交、勿外发。</p>

<div class="card tiles">
<div class="tile"><div class="v">{total}</div><div class="l">Total frames</div></div>
<div class="tile"><div class="v">{len(kept)}</div><div class="l">Kept (written)</div></div>
<div class="tile"><div class="v">{len(attached) + len(transient)}</div><div class="l">Attached (not written)</div></div>
<div class="tile"><div class="v">{rate:.1f}%</div><div class="l">Attach rate</div></div>
<div class="tile"><div class="v">{saved/1048576:.1f} MB</div><div class="l">Disk saved of {total_bytes/1048576:.1f} MB</div></div>
</div>

<h2>Timeline — decision per frame</h2>
<div class="card">
<div class="legend"><span><i class="sw" style="background:var(--kept)"></i>kept</span>
<span><i class="sw" style="background:var(--attached)"></i>attached</span>
<span><i class="sw" style="background:var(--transient)"></i>transient (popup intercepted)</span></div>
{svg_timeline(frames)}
</div>

<h2>Diff fraction distribution (vs anchor)</h2>
<div class="card">{svg_hist(frames)}</div>

<h2>Per-app frames &amp; attach share</h2>
<div class="card">
<div class="legend"><span><i class="sw" style="background:var(--kept)"></i>kept</span>
<span><i class="sw" style="background:var(--attached)"></i>attached</span></div>
{svg_apps(frames)}
</div>

<h2>全部 {total} 帧 — 时间序 + 逐帧判定</h2>
<div class="card">
<div class="legend"><span><i class="sw" style="background:var(--kept)"></i>保留(落盘)</span>
<span><i class="sw" style="background:var(--attached)"></i>附着(不写盘,时间戳记到锚点账上)</span>
<span><i class="sw" style="background:var(--transient)"></i>瞬态弹层拦截(不写盘)</span></div>
{gallery_html}
</div>

<div class="card note">
守卫与兜底统计:标题守卫拦下 {len(guard_kept)} 帧(像素几乎没变但标题变了,强制保留);
K 阀强制落盘 {len(kvalve)} 帧;瞬态弹层拦截 {len(transient)} 帧。<br>
注意:本数据由当前管线采集——键鼠空闲期间 roll 暂停,静止帧占比偏低,
故本附着率是 L1 真实收益的<b>下界</b>;采集解耦落地后静止帧进流,附着率会更高。
</div>
</div></body></html>"""
    with open(out_path, "w", encoding="utf-8") as fh:
        fh.write(html)


def main():
    shot_dir = sys.argv[1]
    db_path = sys.argv[2]
    date_str = os.path.basename(shot_dir.rstrip("/\\"))
    out_path = sys.argv[3] if len(sys.argv) > 3 else os.path.join(
        os.path.dirname(__file__), "output", "l1", f"dedup-eval-{date_str}.html")
    os.makedirs(os.path.dirname(out_path), exist_ok=True)

    # 截图路径 → app/标题(取该路径最早的活动行)
    conn = sqlite3.connect(f"file:{db_path}?mode=ro", uri=True)
    meta = {}
    for p, app, title in conn.execute(
            "SELECT screenshot_path, process_name, window_title FROM activities "
            "WHERE screenshot_path IS NOT NULL AND screenshot_path != '' ORDER BY started_at DESC"):
        meta[os.path.basename(p)] = (app, title)
    conn.close()

    frames = []
    for name in sorted(os.listdir(shot_dir)):
        if not name.endswith(".jpg"):
            continue
        app, title = meta.get(name, (None, None))
        frames.append(Frame(os.path.join(shot_dir, name), app, title))
    if not frames:
        print("目录里没有 jpg 帧")
        return

    run_gate(frames)
    build_html(frames, out_path, date_str)

    kept = sum(1 for f in frames if f.decision == "kept")
    att = sum(1 for f in frames if f.decision in ("attached", "transient"))
    saved = sum(f.size for f in frames if f.decision in ("attached", "transient"))
    print(f"帧数 {len(frames)} | 保留 {kept} | 附着 {att} ({att/len(frames)*100:.1f}%) "
          f"| 免写 {saved/1048576:.1f} MB")
    for r in ("first", "confirmed", "event", "title-guard", "k-valve", "eos"):
        n = sum(1 for f in frames if f.decision == "kept" and f.reason == r)
        if n:
            print(f"  保留原因 {r}: {n}")
    tr = sum(1 for f in frames if f.decision == "transient")
    if tr:
        print(f"  瞬态拦截: {tr}")
    print(f"报告: {out_path}")


if __name__ == "__main__":
    main()
