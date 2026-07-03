#!/usr/bin/env python3
"""
L1 感知哈希 POC(屏幕记忆 · 消化瀑布第一级,见 docs/design/screen-memory.md)。

测"噪音类变化"(鼠标/时钟/输入法候选框)与"真内容变化"(打字/滚动/换页)在
aHash/dHash/pHash 上的汉明距离,寻找 L1 附着阈值。

- 底图取真实 Hindsight 截图(~/Library/Application Support/Hindsight/screenshots/ 最新一天),
  按平均亮度选"最暗 + 最亮"各一张,两种主题各跑一遍全套场景;
- 扰动主题自适应:先采样落点区域的中位背景色铺底,再按背景明暗选字色/面板色——
  避免"暗色主题上贴白底黑字"人为放大距离;
- 每个场景输出原分辨率 A/B 上下堆叠示意图到 scripts/poc/output/l1/(已 gitignore,含真实屏幕内容,勿提交);
- 另扫描当天相邻帧对(间隔≤90s),给出真实底噪分布。

纯 Python + PIL,无 numpy。用法:python3 scripts/poc/l1-phash-poc.py
"""
import math, os, random, sys, glob, time
from PIL import Image, ImageChops, ImageDraw, ImageFont, ImageEnhance

SHOT_ROOT = os.path.expanduser("~/Library/Application Support/Hindsight/screenshots")
OUT_DIR = os.path.join(os.path.dirname(os.path.abspath(__file__)), "output", "l1")

# ---------- 哈希实现 ----------

def gray_grid(img, w, h):
    g = img.convert("L").resize((w, h), Image.LANCZOS)
    return list(g.getdata())

def ahash(img, n=8):
    px = gray_grid(img, n, n)
    avg = sum(px) / len(px)
    return [1 if p > avg else 0 for p in px]

def dhash(img, n=8):
    px = gray_grid(img, n + 1, n)
    bits = []
    for y in range(n):
        row = px[y * (n + 1):(y + 1) * (n + 1)]
        bits += [1 if row[x] < row[x + 1] else 0 for x in range(n)]
    return bits

_DCT_C = {}
def dct_matrix(N):
    if N not in _DCT_C:
        _DCT_C[N] = [[math.cos(math.pi * (2 * x + 1) * u / (2 * N)) for x in range(N)] for u in range(N)]
    return _DCT_C[N]

def phash(img, n=8, size=32):
    px = gray_grid(img, size, size)
    g = [px[y * size:(y + 1) * size] for y in range(size)]
    C = dct_matrix(size)
    tmp = [[sum(C[u][x] * g[x][y] for x in range(size)) for y in range(size)] for u in range(n)]
    d = [[sum(tmp[u][y] * C[v][y] for y in range(size)) for v in range(n)] for u in range(n)]
    flat = [d[u][v] for u in range(n) for v in range(n)]
    med = sorted(flat[1:])[len(flat[1:]) // 2]  # 排除 DC 后取中位数
    return [1 if f > med else 0 for f in flat]

def ham(a, b):
    return sum(x != y for x, y in zip(a, b))

HASHES = [("aHash8", lambda i: ahash(i, 8), 64),
          ("dHash8", lambda i: dhash(i, 8), 64),
          ("pHash8", lambda i: phash(i, 8), 64),
          ("dHash16", lambda i: dhash(i, 16), 256)]

# ---------- 主题工具 ----------

def mean_lum(img):
    g = img.convert("L").resize((16, 16))
    d = list(g.getdata())
    return sum(d) / len(d)

def region_bg(img, box):
    """采样区域中位背景色(逐通道中位数,8×8 降采样)。"""
    crop = img.crop(box).resize((8, 8))
    chans = list(zip(*crop.getdata()))
    return tuple(sorted(c)[len(c) // 2] for c in chans[:3])

def lum_of(rgb):
    return 0.299 * rgb[0] + 0.587 * rgb[1] + 0.114 * rgb[2]

def text_color(bg):
    return (215, 215, 215) if lum_of(bg) < 128 else (25, 25, 25)

# ---------- 扰动构造(全部主题自适应) ----------

def load_font(size):
    for p, idx in [("/System/Library/Fonts/PingFang.ttc", 0),
                   ("/System/Library/Fonts/Hiragino Sans GB.ttc", 0),
                   ("/Library/Fonts/Arial Unicode.ttf", 0)]:
        try:
            return ImageFont.truetype(p, size, index=idx)
        except Exception:
            continue
    return ImageFont.load_default()

def draw_cursor(img, x, y, s=1.0):
    im = img.copy()
    d = ImageDraw.Draw(im)
    P = lambda dx, dy: (x + int(dx * s), y + int(dy * s))
    pts = [P(0, 0), P(0, 19), P(5, 15), P(8, 22), P(11, 21), P(8, 14), P(14, 14)]
    d.polygon(pts, fill=(0, 0, 0), outline=(255, 255, 255))
    return im

def patch_clock(img, text, s=1.0):
    im = img.copy()
    d = ImageDraw.Draw(im)
    w = im.width
    box = (w - int(78 * s), int(3 * s), w - int(18 * s), int(21 * s))
    bg = region_bg(img, box)  # 菜单栏右上角:铺原底色再写字,模拟数字翻动
    d.rectangle(box, fill=bg)
    d.text((w - int(74 * s), int(4 * s)), text,
           font=load_font(max(8, int(13 * s))), fill=text_color(bg))
    return im

def ime_popup(img, fx=0.30, fy=0.52, s=1.0):
    im = img.copy()
    d = ImageDraw.Draw(im)
    x, y = int(im.width * fx), int(im.height * fy)
    box = (x, y, x + int(340 * s), y + int(44 * s))
    dark = lum_of(region_bg(img, box)) < 128
    panel = (45, 45, 45) if dark else (250, 250, 250)
    border = (95, 95, 95) if dark else (180, 180, 180)
    d.rounded_rectangle(box, radius=int(8 * s), fill=panel, outline=border)
    d.text((x + int(12 * s), y + int(10 * s)), "1. 你好  2. 拟好  3. 泥  4. 妮  5. 尼",
           font=load_font(max(8, int(17 * s))), fill=text_color(panel))
    return im

_LINE_TEXTS = [
    "第{k}行:今天实现了感知哈希的像素级查重逻辑测试。",
    "第{k}条:昨晚讨论的会话折叠方案需要重新评估边界。",  # 同长度不同内容,供原地替换用
]

def add_lines(img, n_lines, s=1.0, font_px=14, alt=0):
    """模拟打字:逐行先铺该行区域的原中位底色,再按主题字色写字——不引入异色斑块。
    font_px 为逻辑像素(一般人正文默认 14-16),s=画布px/逻辑px。
    alt 选文案模板:同位置同字号不同 alt = '原地替换'场景。"""
    im = img.copy()
    d = ImageDraw.Draw(im)
    f = load_font(max(6, int(font_px * s)))
    k_f = font_px / 14
    pitch, lh, lw = int((font_px + 8) * s), int((font_px + 6) * s), int(460 * k_f * s)
    x, y0 = int(im.width * 0.22), int(im.height * 0.45)
    for k in range(n_lines):
        box = (x, y0 + k * pitch, x + lw, y0 + k * pitch + lh)
        bg = region_bg(img, box)
        d.rectangle(box, fill=bg)
        d.text((x, y0 + k * pitch), _LINE_TEXTS[alt].format(k=k + 1),
               font=f, fill=text_color(bg))
    return im

def chat_lines(img, n_lines, s=1.0, font_px=12):
    """角落聊天窗新增 1-2 行消息:小面积但持久的真实新内容(硬正类)。"""
    im = img.copy()
    d = ImageDraw.Draw(im)
    f = load_font(max(6, int(font_px * s)))
    k_f = font_px / 12
    pitch, lh, lw = int((font_px + 8) * s), int((font_px + 6) * s), int(300 * k_f * s)
    x, y0 = int(im.width * 0.68), int(im.height * 0.80)
    for k in range(n_lines):
        box = (x, y0 + k * pitch, x + lw, y0 + k * pitch + lh)
        bg = region_bg(img, box)
        d.rectangle(box, fill=bg)
        d.text((x, y0 + k * pitch), f"李四:那个方案第{k+3}点我有不同意见,晚点细聊",
               font=f, fill=text_color(bg))
    return im

def scroll(img, dy):
    im = img.copy()
    w, h, top = im.width, im.height, 30
    body = im.crop((0, top + dy, w, h))
    im.paste(body, (0, top))
    band = img.crop((0, top + 120, w, top + 120 + dy))  # 底部滚入"新内容"
    im.paste(band, (0, h - dy))
    return im

def notif_banner(img, s=1.0):
    """系统通知横幅(右上角)——瞬态弹层,负类:采样层保证不了它,
    消息若被用户在意会在源 app 会话中被正式捕获(与 IME 同判据)。"""
    im = img.copy()
    d = ImageDraw.Draw(im)
    w = im.width
    box = (w - int(392 * s), int(34 * s), w - int(24 * s), int(122 * s))
    dark = lum_of(region_bg(img, box)) < 128
    panel = (52, 52, 54) if dark else (246, 246, 246)
    d.rounded_rectangle(box, radius=int(12 * s), fill=panel, outline=(120, 120, 120))
    d.text((box[0] + int(14 * s), box[1] + int(12 * s)), "微信",
           font=load_font(max(8, int(15 * s))), fill=text_color(panel))
    d.text((box[0] + int(14 * s), box[1] + int(38 * s)),
           "张三:今晚的会议改到八点了,记得带上季度报表",
           font=load_font(max(8, int(13 * s))), fill=text_color(panel))
    return im

# ---------- 帧差法(L1 定案算法) ----------

FD_SIZE = (256, 144)   # 缩略尺寸
FD_TAU = 12            # 像素级阈值:|Δ|>τ 才算变化(滤 JPEG 纹波/字体平滑抖动)

def fd_thumb(img):
    return img.convert("L").resize(FD_SIZE, Image.BILINEAR)

def frame_diff(ta, tb):
    """变化像素占比。输入为 fd_thumb 的缩略灰度。全程 PIL C 路径。"""
    hist = ImageChops.difference(ta, tb).histogram()
    changed = sum(hist[FD_TAU + 1:])
    return changed / (FD_SIZE[0] * FD_SIZE[1])

def fd_eval(dark, light):
    """帧差法标定:各类型的 fraction 分布 → 按'正类误附着=0'约束选 f。"""
    for cls_name, logical_h in RES_CLASSES:
        s = dark.height / logical_h
        pairs = build_pairs(dark, light, s)
        rows = [(lab, typ, frame_diff(fd_thumb(a), fd_thumb(b))) for lab, typ, a, b in pairs]
        pos = sorted(f for lab, _, f in rows if lab)
        neg = sorted(f for lab, _, f in rows if not lab)
        print(f"\n===== {cls_name} =====")
        print(f"{'类型':<14} 标签  fraction: 最小 / 中位 / 最大")
        for typ in ("identical", "mouse", "clock", "ime", "dim", "notif", "type1-4行",
                    "type5-10行", "scroll", "page_switch", "chat+1-2行", "replace5-10行"):
            fs = sorted(f for lab, t, f in rows if t == typ)
            lab = next(l for l, t, _ in rows if t == typ)
            print(f"  {typ:<14} {'正' if lab else '负'}   "
                  f"{fs[0]*100:6.3f}% / {fs[len(fs)//2]*100:6.3f}% / {fs[-1]*100:6.3f}%")
        f_np = pos[0]  # 正类最小值:f 必须低于它才能召回 100%
        for f_op in (f_np * 0.5, f_np * 0.8):
            att = sum(1 for x in neg if x < f_op)
            print(f"  运营点 f={f_op*100:.3f}%(正类最小值×{f_op/f_np:.1f}):"
                  f"正类召回 100%,负类附着率 {att}/{len(neg)} = {att*100//len(neg)}%")

F_FINAL = 0.0010  # 定案运营点:0.10% 单值(用户裁定放宽,非 100% 召回)
TRANSIENT_TYPES = {"ime", "notif"}  # 瞬态弹层:下一帧(30s后)必然消失

def fd_final(dark, light):
    """定案配置仿真:f=0.10% + 持久性确认(帧 B 保留 ⟺ diff(B,A)≥f 且 diff(C,A)≥f,
    C=下一帧:瞬态类回到底图 A,持久类维持 B)。K 阀是序列机制,不在图对仿真范围。"""
    print(f"定案配置:f={F_FINAL*100:.2f}% 单值 + 持久性确认(瞬态弹层一帧后消失)\n")
    for cls_name, logical_h in RES_CLASSES:
        s = dark.height / logical_h
        pairs = build_pairs(dark, light, s)
        stats = {}  # typ -> [lab, kept, total]
        for lab, typ, a, b in pairs:
            ta, tb = fd_thumb(a), fd_thumb(b)
            tc = ta if typ in TRANSIENT_TYPES else tb  # 下一帧
            kept = frame_diff(tb, ta) >= F_FINAL and frame_diff(tc, ta) >= F_FINAL
            st = stats.setdefault(typ, [lab, 0, 0])
            st[1] += kept
            st[2] += 1
        pos_k = sum(k for lab, k, n in stats.values() if lab)
        pos_n = sum(n for lab, k, n in stats.values() if lab)
        neg_a = sum(n - k for lab, k, n in stats.values() if not lab)
        neg_n = sum(n for lab, k, n in stats.values() if not lab)
        print(f"===== {cls_name} =====")
        print(f"  正类召回(该保留的保留了) {pos_k}/{pos_n} = {pos_k*100//pos_n}%")
        print(f"  负类召回(该拦截的拦截了) {neg_a}/{neg_n} = {neg_a*100//neg_n}%")
        for typ, (lab, k, n) in stats.items():
            ok = k if lab else n - k
            flag = "" if ok == n else ("  ← 全漏" if ok == 0 else f"  ← 漏 {n-ok}")
            print(f"    {typ:<14} {'正' if lab else '负'}  {ok:>2}/{n}{flag}")
        print()

# ---------- 判定效果画廊(--gallery,本地对比图) ----------

C_ATTACH = (91, 127, 157)   # 拦截=石板蓝
C_KEEP = (198, 106, 32)     # 保留=琥珀
C_TRANS = (13, 130, 120)    # 瞬态丢弃=青

def compose_row(imgs, header, rgb, path, gap=8):
    """横排原分辨率拼图 + 顶部判定横幅,烙进图片。"""
    h = max(i.height for i in imgs)
    w = sum(i.width for i in imgs) + gap * (len(imgs) - 1)
    canvas = Image.new("RGB", (w, h + 48), (24, 26, 28))
    d = ImageDraw.Draw(canvas)
    d.rectangle((0, 0, w, 48), fill=rgb)
    d.text((16, 11), header, font=load_font(22), fill=(255, 255, 255))
    x = 0
    for im in imgs:
        canvas.paste(im, (x, 48))
        x += im.width + gap
    canvas.save(path, quality=88)

def gallery(shots, dark, light):
    gdir = os.path.join(OUT_DIR, "gallery")
    os.makedirs(gdir, exist_ok=True)
    files = {"real": [], "syn": [], "persist": []}

    # ① 真实流:相邻帧按定案配置判定,拦截/保留各挑代表
    def t_of(p):
        s = os.path.basename(p)[:6]
        return f"{s[:2]}:{s[2:4]}:{s[4:6]}"
    recent = shots[-120:]
    pairs = []
    prev = None
    for p in recent:
        img = Image.open(p).convert("RGB")
        if prev is not None:
            fr = frame_diff(fd_thumb(prev[1]), fd_thumb(img))
            pairs.append((fr, prev[0], p, prev[1], img))
        prev = (p, img)
    att = sorted([x for x in pairs if x[0] < F_FINAL])[:8]
    kept = sorted([x for x in pairs if x[0] >= F_FINAL])
    kept = kept[:3] + kept[len(kept)//2 - 1:len(kept)//2 + 2] + kept[-3:]
    for i, (fr, pa, pb, ia, ib) in enumerate(att + kept):
        is_att = fr < F_FINAL
        name = f"real_{i:02d}_{'attach' if is_att else 'keep'}.jpg"
        compose_row([ia, ib],
                    f"真实流 · {t_of(pa)} → {t_of(pb)} · 变化占比 {fr*100:.3f}%"
                    f" · {'拦截(不落盘)' if is_att else '保留'}",
                    C_ATTACH if is_att else C_KEEP, os.path.join(gdir, name))
        files["real"].append(name)

    # ② 合成场景:定案配置逐场景判定
    for tag, base in (("dark", dark), ("light", light)):
        other = light if base is dark else dark
        scen = [
            ("mouse", "光标移动", draw_cursor(base, 300, 250), draw_cursor(base, 900, 450), False),
            ("clock", "时钟翻分", patch_clock(base, "14:23"), patch_clock(base, "14:24"), False),
            ("ime", "输入法候选框", base, ime_popup(base), True),
            ("notif", "通知横幅", base, notif_banner(base), True),
            ("type3", "打字 +3 行", base, add_lines(base, 3), False),
            ("type8", "打字 +8 行", base, add_lines(base, 8), False),
            ("replace8", "原地替换 8 行", add_lines(base, 8, alt=0), add_lines(base, 8, alt=1), False),
            ("scroll", "滚动 200px", base, scroll(base, 200), False),
            ("switch", "整页切换", base, other, False),
        ]
        for slug, label, a, b, transient in scen:
            fr = frame_diff(fd_thumb(a), fd_thumb(b))
            if transient:
                verdict, rgb = "瞬态 → 持久性确认丢弃", C_TRANS
            elif fr < F_FINAL:
                verdict, rgb = "拦截(不落盘)", C_ATTACH
            else:
                verdict, rgb = "保留", C_KEEP
            name = f"syn_{tag}_{slug}.jpg"
            compose_row([a, b], f"{label} · 变化占比 {fr*100:.3f}% · {verdict}",
                        rgb, os.path.join(gdir, name))
            files["syn"].append(name)

    # ③ 持久性确认:三帧序列(锚点 → 挂起帧 → 下一帧)
    seqs = [
        ("ime", "输入法候选框弹出又消失", dark, ime_popup(dark), dark,
         "挂起 → 下一帧变化消失 → 丢弃(不落盘)", C_TRANS),
        ("notif", "通知横幅弹出又消失", light, notif_banner(light), light,
         "挂起 → 下一帧变化消失 → 丢弃(不落盘)", C_TRANS),
        ("type", "打字 5 行并持续", dark, add_lines(dark, 5), add_lines(dark, 5),
         "挂起 → 下一帧变化仍在 → 确认保留", C_KEEP),
        ("replace", "原地替换 8 行并持续", add_lines(light, 8, alt=0), add_lines(light, 8, alt=1),
         add_lines(light, 8, alt=1), "挂起 → 下一帧变化仍在 → 确认保留", C_KEEP),
    ]
    for slug, label, a, b, c, verdict, rgb in seqs:
        fr = frame_diff(fd_thumb(a), fd_thumb(b))
        name = f"persist_{slug}.jpg"
        compose_row([a, b, c], f"{label} · 占比 {fr*100:.3f}% · {verdict}",
                    rgb, os.path.join(gdir, name))
        files["persist"].append(name)

    # 索引页
    secs = [("真实流实拍判定(f=0.10%)", "real"), ("合成场景", "syn"), ("持久性确认(三帧)", "persist")]
    html = ["<title>L1 判定效果 · 对比图</title><style>",
            "body{background:#16191c;color:#dfe5e5;font-family:'PingFang SC',sans-serif;",
            "max-width:1400px;margin:0 auto;padding:24px}h2{margin:36px 0 12px}",
            "img{width:100%;border-radius:6px;margin:10px 0;display:block}</style>",
            "<h1>L1 静止门 · 判定效果对比图</h1>",
            "<p>顶栏即判定:蓝=拦截(不落盘) 琥珀=保留 青=瞬态丢弃。图片全部在本机,未上传。</p>"]
    for title, key in secs:
        html.append(f"<h2>{title}</h2>")
        html += [f'<img src="{n}" loading="lazy">' for n in files[key]]
    idx = os.path.join(gdir, "index.html")
    open(idx, "w").write("\n".join(html))
    print(f"画廊:{len(files['real'])+len(files['syn'])+len(files['persist'])} 张对比图 → {idx}")
    return idx

def fd_bench(shots):
    """资源基准:批量路径(JPEG解码+缩略+帧差)与抓拍路径(仅缩略+帧差)。"""
    paths = shots[-31:]
    t0 = time.perf_counter()
    thumbs = [fd_thumb(Image.open(p).convert("RGB")) for p in paths]
    t_full = (time.perf_counter() - t0) / len(paths) * 1000
    imgs = [Image.open(p).convert("RGB") for p in paths]  # 预解码,测抓拍路径
    t0 = time.perf_counter()
    thumbs2 = [fd_thumb(im) for im in imgs]
    t_thumb = (time.perf_counter() - t0) / len(imgs) * 1000
    t0 = time.perf_counter()
    n = 0
    for a, b in zip(thumbs2, thumbs2[1:]):
        frame_diff(a, b); n += 1
    t_diff = (time.perf_counter() - t0) / max(1, n) * 1000
    import sys as _s
    mem = FD_SIZE[0] * FD_SIZE[1]  # 锚点字节数(L 模式 1B/px)
    print(f"\n—— 资源基准(n={len(paths)},真实存档图)——")
    print(f"批量路径:JPEG解码+缩略      {t_full:6.2f} ms/帧")
    print(f"抓拍路径:缩略(帧已在内存)    {t_thumb:6.2f} ms/帧")
    print(f"帧差本体(直方图统计)        {t_diff:6.3f} ms/对")
    print(f"常驻状态:锚点缩略图         {mem/1024:.0f} KB/屏")

# ---------- 精准率/召回率评测(--pr) ----------

# 逻辑高度档:同一 UI 元素(逻辑像素恒定)在不同屏上的占屏比不同。
# 675 ≈ 现有存档画布 1:1;1080 ≈ 主流 1080p 1× / 4K@2×;1440 ≈ QHD 1× / 4K@150%。
RES_CLASSES = [("675逻辑(小屏/高缩放)", 675), ("1080逻辑(1080p/4K@2x)", 1080),
               ("1440逻辑(QHD@1x)", 1440)]

def build_pairs(dark, light, s):
    """带标签图对。标签标准:变化里是否有'值得记住的用户活动信息'。
    负类=瞬态/环境/机器自走;正类=持久内容变化(打字≥5行、滚动、换页、聊天新消息)。"""
    random.seed(42)
    pairs = []  # (label正类?, 类型, imgA, imgB)
    for base in (dark, light):
        w, h = base.size
        for _ in range(15):
            rf = lambda a, b: a + random.random() * (b - a)
            # 负类:瞬态/环境
            pairs.append((False, "identical", base, base))
            pairs.append((False, "mouse",
                          draw_cursor(base, int(w*rf(.1,.8)), int(h*rf(.1,.8)), s),
                          draw_cursor(base, int(w*rf(.1,.8)), int(h*rf(.1,.8)), s)))
            h1, m1 = random.randint(0, 23), random.randint(0, 58)
            pairs.append((False, "clock", patch_clock(base, f"{h1:02d}:{m1:02d}", s),
                                          patch_clock(base, f"{h1:02d}:{m1+1:02d}", s)))
            pairs.append((False, "ime", base, ime_popup(base, rf(.15, .6), rf(.3, .7), s)))
            pairs.append((False, "dim", base,
                          ImageEnhance.Brightness(base).enhance(1 - rf(.02, .08))))
            pairs.append((False, "notif", base, notif_banner(base, s)))
            # 负类:打字 1-4 行(用户裁定:不足 5 行不算新数据,靠漂移累积)
            pairs.append((False, "type1-4行", base,
                          add_lines(base, random.randint(1, 4), s, random.randint(14, 16))))
            # 正类:持久内容变化(正文字号取一般人默认 14-16 随机)
            pairs.append((True, "type5-10行", base,
                          add_lines(base, random.randint(5, 10), s, random.randint(14, 16))))
            pairs.append((True, "scroll", base, scroll(base, int(random.randint(40, 400) * s))))
            pairs.append((True, "page_switch", base, light if base is dark else dark))
            pairs.append((True, "chat+1-2行", base,
                          chat_lines(base, random.randint(1, 2), s, random.randint(12, 13))))
            # 正类:原地替换——行数不变,同位置同字号,内容全换(编辑/刷新场景,最难)
            n_r, f_r = random.randint(5, 10), random.randint(14, 16)
            pairs.append((True, "replace5-10行",
                          add_lines(base, n_r, s, f_r, alt=0),
                          add_lines(base, n_r, s, f_r, alt=1)))
    return pairs

def pr_eval(dark, light):
    for cls_name, logical_h in RES_CLASSES:
        s = dark.height / logical_h  # 画布px / 逻辑px
        pairs = build_pairs(dark, light, s)
        dists = [(lab, typ, ham(dhash(a, 16), dhash(b, 16))) for lab, typ, a, b in pairs]
        print(f"\n===== {cls_name}  (s={s:.2f}, 14px字≈占屏高 {14/logical_h*100:.1f}%) =====")
        print("阈值   下传召回率  下传精准率  误附着(信息延迟)  误下传(白跑OCR)")
        for th in (4, 6, 8, 10, 12):
            tp = sum(1 for lab, _, d in dists if lab and d > th)
            fn = sum(1 for lab, _, d in dists if lab and d <= th)
            fp = sum(1 for lab, _, d in dists if not lab and d > th)
            tn = sum(1 for lab, _, d in dists if not lab and d <= th)
            mark = "  ← 候选" if th == 8 else ""
            print(f" >{th:<3} {tp/max(1,tp+fn)*100:>8.1f}%  {tp/max(1,tp+fp)*100:>8.1f}%"
                  f"   {fn:>4}/{tp+fn}          {fp:>4}/{fp+tn}{mark}")
        print(f"各类型 @阈值8(距离范围/中位/正确率):")
        for typ in ("identical", "mouse", "clock", "ime", "dim", "notif", "type1-4行",
                    "type5-10行", "scroll", "page_switch", "chat+1-2行", "replace5-10行"):
            sub = [(lab, d) for lab, t, d in dists if t == typ]
            lab = sub[0][0]
            ok = sum(1 for l, d in sub if (d > 8) == l)
            ds = sorted(d for _, d in sub)
            print(f"  {typ:<12} {'正' if lab else '负'}  {ok:>2}/{len(sub)}  "
                  f"{ds[0]}-{ds[-1]} 中位{ds[len(ds)//2]}")

# ---------- 示意图输出 ----------

def save_pair(tag, idx, slug, name, a, b):
    # 原分辨率,上下堆叠(上=A 下=B),不缩放
    gap, bar = 12, 30
    canvas = Image.new("RGB", (a.width, a.height * 2 + gap + bar), (30, 30, 30))
    canvas.paste(a, (0, bar))
    canvas.paste(b, (0, bar + a.height + gap))
    d = ImageDraw.Draw(canvas)
    d.text((8, 6), f"[{tag}] {name}   [上=A 下=B]", font=load_font(16), fill=(255, 255, 255))
    canvas.save(os.path.join(OUT_DIR, f"{tag}_{idx:02d}_{slug}.jpg"), quality=90)

# ---------- 主流程 ----------

def run_suite(tag, base, other):
    scen = [
        ("mouse_appear",  "鼠标出现(无→有)", base, draw_cursor(base, int(base.width*.3), int(base.height*.4))),
        ("mouse_move",    "鼠标移动(左上→右下)", draw_cursor(base, int(base.width*.3), int(base.height*.4)),
                                             draw_cursor(base, int(base.width*.7), int(base.height*.6))),
        ("clock_tick",    "菜单栏时钟 14:23→14:24", patch_clock(base, "14:23"), patch_clock(base, "14:24")),
        ("ime_popup",     "输入法候选框弹出", base, ime_popup(base)),
        ("type_1line",    "打字 +1 行", base, add_lines(base, 1)),
        ("type_3lines",   "打字 +3 行", base, add_lines(base, 3)),
        ("type_8lines",   "打字 +8 行(一段)", base, add_lines(base, 8)),
        ("scroll_60",     "滚动 60px", base, scroll(base, 60)),
        ("scroll_200",    "滚动 200px", base, scroll(base, 200)),
        ("dim_7pct",      "屏幕变暗 7%", base, ImageEnhance.Brightness(base).enhance(0.93)),
        ("page_switch",   "整页切换(另一张真图)", base, other),
    ]
    cols = "  ".join(f"{n:>8}" for n, _, bits in HASHES)
    print(f"\n=== 底图[{tag}] 平均亮度 {mean_lum(base):.0f}/255 ===")
    print(f"{'场景':<24}{cols}   (括号=按 64-bit 归一)")
    for i, (slug, name, a, b) in enumerate(scen, 1):
        vals = []
        for hn, fn, bits in HASHES:
            d = ham(fn(a), fn(b))
            vals.append(f"{d:>4}({d * 64 // bits:>2})" if bits != 64 else f"{d:>8}")
        print(f"{name:<24}" + "  ".join(vals))
        save_pair(tag, i, slug, name, a, b)

def main():
    days = sorted(d for d in glob.glob(os.path.join(SHOT_ROOT, "*")) if os.path.isdir(d))
    pr_only = "--pr" in sys.argv
    if not days:
        sys.exit(f"找不到截图目录:{SHOT_ROOT}")
    shots = sorted(glob.glob(os.path.join(days[-1], "*.jpg")))
    if len(shots) < 2:
        sys.exit("需要至少两张真实截图")
    os.makedirs(OUT_DIR, exist_ok=True)

    # 按平均亮度挑最暗/最亮两张真实底图(采样最近 60 张)
    sample = shots[-60:]
    lums = [(mean_lum(Image.open(p).convert("RGB")), p) for p in sample]
    lums.sort()
    dark_p, light_p = lums[0][1], lums[-1][1]
    dark = Image.open(dark_p).convert("RGB")
    light = Image.open(light_p).convert("RGB")
    print(f"暗色底图: {os.path.basename(dark_p)} (亮度{lums[0][0]:.0f})   "
          f"亮色底图: {os.path.basename(light_p)} (亮度{lums[-1][0]:.0f})")

    if "--gallery" in sys.argv:
        gallery(shots, dark, light)
        return

    if "--final" in sys.argv:
        fd_final(dark, light)
        return

    if "--fd" in sys.argv:
        fd_eval(dark, light)
        fd_bench(shots)
        # 真实流:帧差在今天相邻帧上的附着率(f 取 0.3%)
        recent = shots[-150:]
        prev, att, tot = None, 0, 0
        for p in recent:
            t = fd_thumb(Image.open(p).convert("RGB"))
            if prev is not None:
                tot += 1
                if frame_diff(prev, t) < 0.003:
                    att += 1
            prev = t
        print(f"真实相邻帧(n={tot}):f=0.3% 时附着率 {att*100//max(1,tot)}%")
        return

    if pr_only:
        pr_eval(dark, light)
        # 真实案例:同窗口大幅滚动(test3/4)与不同页面(test1/2)必须判"下传"
        tdir = os.path.join(os.path.dirname(OUT_DIR))
        for a, b, desc in [("test3", "test4", "同窗口大幅滚动"), ("test1", "test2", "不同页面")]:
            pa, pb = os.path.join(tdir, f"{a}.png"), os.path.join(tdir, f"{b}.png")
            if os.path.exists(pa) and os.path.exists(pb):
                d = ham(dhash(Image.open(pa).convert("RGB"), 16),
                        dhash(Image.open(pb).convert("RGB"), 16))
                print(f"\n真实案例 {a}↔{b}({desc}): dHash16 距离 {d}/256 → "
                      f"{'下传 ✓' if d > 8 else '误附着 ✗'}")
        return

    print(f"示意图输出: {OUT_DIR}")
    run_suite("dark", dark, light)   # 换页对照用相反主题(最大反差)
    run_suite("light", light, dark)

    # 真实相邻帧底噪分布(最近 150 张,间隔 ≤90s 的对)
    print("\n—— 真实相邻帧底噪(dHash8 / dHash16,间隔≤90s)——")
    recent = shots[-150:]
    def t_of(p):
        s = os.path.basename(p)[:6]
        return int(s[:2]) * 3600 + int(s[2:4]) * 60 + int(s[4:6])
    pairs, d8s, d16s, attach = 0, [], [], []
    prev_img, prev_t, prev_p = None, None, None
    for p in recent:
        t = t_of(p)
        img = Image.open(p).convert("RGB")
        if prev_img is not None and t - prev_t <= 90:
            d8 = ham(dhash(prev_img, 8), dhash(img, 8))
            d16 = ham(dhash(prev_img, 16), dhash(img, 16))
            d8s.append(d8); d16s.append(d16); pairs += 1
            if d16 <= 8:
                attach.append((d8, d16, os.path.basename(prev_p), os.path.basename(p)))
        prev_img, prev_t, prev_p = img, t, p
    if pairs:
        for name, arr in [("dHash8", d8s), ("dHash16", d16s)]:
            s = sorted(arr)
            pct = lambda q: s[min(len(s) - 1, int(q * len(s)))]
            print(f"{name}: n={pairs}  p10={pct(.10)} p25={pct(.25)} p50={pct(.50)} "
                  f"p75={pct(.75)} p90={pct(.90)} max={s[-1]}")
        low = sum(1 for d in d16s if d <= 8)
        print(f"dHash16≤8(将被 L1 附着)的相邻对: {low}/{pairs} = {low*100//pairs}%")
        print("样例(dHash16≤8 的对,最多 5 个):")
        for d8, d16, a, b in attach[:5]:
            print(f"  d8={d8} d16={d16}  {a} → {b}")

if __name__ == "__main__":
    main()
