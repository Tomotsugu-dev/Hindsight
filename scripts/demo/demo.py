#!/usr/bin/env python3
"""
演示模式:克隆真实数据 → 全字段净化 → 用隔离数据目录启动 app。

    npm run demo          # 生成(如缺)并用「已安装的正式版 app」启动演示
    npm run demo:fresh    # 删旧演示数据重建,再启动
    python3 scripts/demo/demo.py --gen-only          # 只生成不启动
    python3 scripts/demo/demo.py --dev               # 用 tauri dev 启动(不默认,
                                                     #  不占用你的日常 npm run dev 流程)

原理(零产品代码改动、真实库只读):
- app 原生支持 HINDSIGHT_DATA_DIR 环境变量重定向数据根(bootstrap.rs);
- 本脚本把真实主库/记忆库以只读方式 VACUUM INTO 克隆到 <数据根>/demo/,
  然后对克隆体净化:窗口标题/OCR 文本/日报内容/Chat 历史全部替换为内置
  虚构语料(同一原值→同一假值,时间线连贯);凭据/同步状态/设备表清空;
  截图指针全部置空(真实截图永不进入演示档案);
- 图表节奏、时段分布、app 占比保持真实使用的形状,但每个字符串都是编的;
  app 名单按用户决定保留原值(Cursor/Chrome 等,演示更真)。

注意:app 是单实例的,启动演示前请先从托盘退出正在运行的 Hindsight。

隐私自查:净化后抽样断言"任何原始标题都不再出现",失败即中止。
"""

import argparse
import hashlib
import json
import os
import platform
import shutil
import sqlite3
import subprocess
import sys
from pathlib import Path

# ───────────────────────── 虚构语料 ─────────────────────────

TITLES = {
    "code": [
        "Aurora 数据管线 — main.rs",
        "Aurora 数据管线 — pipeline.py",
        "billing-service — invoice.ts",
        "Aurora 控制台 — Dashboard.tsx",
        "notify-worker — queue.go",
        "Aurora 数据管线 — schema.sql",
        "infra-scripts — deploy.sh",
        "Aurora 控制台 — theme.css",
    ],
    "browser": [
        "Rust 异步运行时对比 — 技术周刊",
        "SQLite WAL 模式详解 — 开发者博客",
        "Aurora 项目周报 - Google Docs",
        "机械键盘选购指南 — 值得买",
        "Figma 组件库最佳实践",
        "PostgreSQL 分区表实战 — 掘金",
        "「星野」新专辑首发 — bilibili",
        "东京五日游攻略 — 马蜂窝",
        "Tauri 2.0 发布说明",
        "GitHub - aurora-lab/pipeline",
    ],
    "chat": [
        "Aurora 项目组",
        "产品评审群",
        "李维(设计)",
        "周报提醒",
        "运维值班群",
        "陈晨",
    ],
    "doc": [
        "季度复盘.docx",
        "Aurora 架构设计.md",
        "面试题库整理",
        "旅行清单",
        "读书笔记 — 《数据密集型应用》",
        "会议纪要 2026-07",
    ],
    "media": [
        "星际旅人 4K — 播放器",
        "Lo-fi 工作歌单",
        "纪录片:深海 — 第 2 集",
        "白噪音 — 雨声",
    ],
    "terminal": [
        "cargo build --release",
        "npm run tauri dev",
        "git rebase -i HEAD~3",
        "htop",
        "ssh aurora-staging",
        "tail -f service.log",
    ],
    "default": [
        "Aurora 项目资料",
        "本周待办",
        "收件箱",
        "设置",
        "快速笔记",
    ],
}

BUCKETS = [
    ("code", ["cursor", "code", "idea", "zed", "studio", "vim", "sublime"]),
    ("browser", ["chrome", "safari", "edge", "firefox", "arc", "brave", "browser"]),
    ("chat", ["wechat", "weixin", "slack", "discord", "telegram", "qq", "teams", "lark", "dingtalk"]),
    ("doc", ["word", "pages", "notion", "obsidian", "typora", "onenote", "docs"]),
    ("media", ["music", "spotify", "iina", "vlc", "quicktime", "player", "netease"]),
    ("terminal", ["terminal", "iterm", "warp", "alacritty", "kitty", "powershell", "cmd"]),
]

OCR_PARAGRAPHS = [
    "Aurora 数据管线 v2 重构要点:摄取层改为增量拉取,水位线落库;"
    "转换层拆出独立 worker,失败重试上限 3 次;昨日全量回放耗时 42 分钟。",
    "订单确认:Keychron K8 机械键盘(茶轴,国际版),订单号 AUR-2026-0713,"
    "实付 ¥399.00,预计 7 月 15 日送达。收货后记得试一下热插拔轴体。",
    "会议纪要:确认 Q3 目标为管线延迟 P95 < 5 分钟;风险项是上游接口限流,"
    "由李维跟进配额申请;下次评审 7 月 18 日。",
    "面试准备:重点复习一致性哈希、LSM-Tree 写放大、Raft 日志复制;"
    "候选项目讲 Aurora 的幂等消费设计。",
    "东京行程草稿:D1 浅草寺-晴空塔,D2 三鹰之森吉卜力(记得提前一个月抢票),"
    "D3 镰仓一日游;酒店定在上野附近,方便坐京成线。",
    "调试记录:notify-worker 在高并发下偶发重复推送,根因是去重键没包含渠道字段;"
    "修复后压测 10 万条无重复,准备灰度。",
]

SEGMENT_SUMMARIES = [
    "上午的时间主要投入在 Aurora 数据管线的重构上:先在 Cursor 中调整了摄取层的"
    "增量拉取逻辑,随后用终端跑了两轮回放验证,中间穿插查阅了几篇关于 SQLite WAL "
    "的资料。整体专注度较高,只有零星的群消息打断。",
    "下午以会议和文档为主:参加了 Q3 目标评审,会后整理会议纪要并更新了架构设计"
    "文档;晚些时候回到编辑器处理 notify-worker 的重复推送问题,定位到去重键缺少"
    "渠道字段并完成修复。",
    "晚间节奏放缓:浏览了机械键盘的选购内容并下了单,之后听着歌单整理了东京行程"
    "草稿,睡前把明天的待办列了出来。",
]

# 注入的可提问素材:B 站标题(晚间浏览器时段)与 Hindsight 开发标题(代码时段),
# 让「看了 B 站多久/几次」「Hindsight 开发花了多久」这类问题有真数据可查。
BILI_TITLES = [
    "【硬核】Rust 所有权,一个视频讲透_哔哩哔哩_bilibili",
    "这个 UI 细节,苹果研究了十年_哔哩哔哩_bilibili",
    "【4K】东京 CityWalk 雨夜漫步_哔哩哔哩_bilibili",
    "程序员的一天 Vlog|远程办公_哔哩哔哩_bilibili",
    "SQLite 是怎么做到零配置的_哔哩哔哩_bilibili",
    "【整活】用 Excel 跑神经网络_哔哩哔哩_bilibili",
    "机械键盘轴体横评:茶轴还是红轴_哔哩哔哩_bilibili",
    "「星野」新专辑全曲解析_哔哩哔哩_bilibili",
]
HINDSIGHT_TITLES = [
    "Hindsight — capture/service.rs",
    "Hindsight — memory/digest.rs",
    "Hindsight — ChatView.tsx",
    "Hindsight — screen-memory.md",
    "Hindsight — ai/ocr.rs",
    "Hindsight — SettingsPage.tsx",
]

# ───────────────────────── 工具 ─────────────────────────


def pick(pool, key):
    h = int(hashlib.md5(key.encode("utf-8")).hexdigest(), 16)
    return pool[h % len(pool)]


def bucket_of(process_name):
    p = (process_name or "").lower()
    for name, kws in BUCKETS:
        if any(k in p for k in kws):
            return name
    return "default"


def fake_title(process_name, original):
    return pick(TITLES[bucket_of(process_name)], f"{process_name}|{original}")


def config_dir():
    if platform.system() == "Darwin":
        return Path.home() / "Library" / "Application Support" / "Hindsight"
    if platform.system() == "Windows":
        return Path(os.environ.get("APPDATA", "")) / "Hindsight"
    return Path.home() / ".config" / "Hindsight"


def data_root():
    env = os.environ.get("HINDSIGHT_DATA_DIR", "").strip()
    if env:
        return Path(env)
    boot = config_dir() / "bootstrap.json"
    if boot.is_file():
        try:
            custom = json.loads(boot.read_text(encoding="utf-8")).get("data_path")
            if custom and str(custom).strip():
                return Path(custom)
        except Exception:
            pass
    return config_dir()  # 默认数据根与配置目录同址


def active_uid():
    f = config_dir() / "active_user.json"
    if not f.is_file():
        return None
    try:
        uid = json.loads(f.read_text(encoding="utf-8")).get("uid")
        return uid if uid else None
    except Exception:
        return None


def db_names(uid):
    if uid:
        return f"hindsight.{uid}.sqlite", f"hindsight-memory.{uid}.sqlite"
    return "hindsight.sqlite", "hindsight-memory.sqlite"


def clone_ro(src, dest):
    dest.parent.mkdir(parents=True, exist_ok=True)
    if dest.exists():
        dest.unlink()
    conn = sqlite3.connect(f"file:{src}?mode=ro", uri=True)
    try:
        conn.execute("VACUUM INTO ?", (str(dest),))
    finally:
        conn.close()


# ───────────────────────── 净化:主库 ─────────────────────────


def sanitize_main(db, demo_root):
    conn = sqlite3.connect(db)
    cur = conn.cursor()
    originals = [
        r[0]
        for r in cur.execute(
            "SELECT DISTINCT window_title FROM activities "
            "WHERE window_title IS NOT NULL AND window_title != '' LIMIT 50"
        )
    ]

    # 活动:标题→语料;截图指针/哈希清空(真实截图绝不进演示档案)
    rows = cur.execute(
        "SELECT id, process_name, window_title FROM activities WHERE window_title IS NOT NULL"
    ).fetchall()
    cur.executemany(
        "UPDATE activities SET window_title = ? WHERE id = ?",
        [(fake_title(p, t), i) for i, p, t in rows],
    )
    cur.execute("UPDATE activities SET screenshot_path = NULL, image_hash = NULL")

    # 注入演示素材:晚间浏览器的 1/3 → B 站;代码类的 2/3 → Hindsight 开发
    rows = cur.execute(
        "SELECT id, process_name, local_hour FROM activities WHERE window_title IS NOT NULL"
    ).fetchall()
    bili, hs = [], []
    for i, proc, lh in rows:
        b = bucket_of(proc)
        if b == "browser" and lh is not None and 18 <= lh <= 23 and i % 2 == 0:
            bili.append((pick(BILI_TITLES, f"b{i}"), i))
        elif b == "code" and i % 3 != 2:
            hs.append((pick(HINDSIGHT_TITLES, f"h{i}"), i))
    cur.executemany("UPDATE activities SET window_title = ? WHERE id = ?", bili)
    cur.executemany("UPDATE activities SET window_title = ? WHERE id = ?", hs)

    # 凭据 / 同步状态:全清。devices 不能删——应用页"本机/设备"列
    # 靠它连接每台设备的条目,删了整页断链;保留行、只脱敏设备名。
    for table in ("auth_state", "sync_outbox", "sync_cursor"):
        cur.execute(f"DELETE FROM {table}")
    devs = cur.execute(
        "SELECT device_id FROM devices ORDER BY is_self DESC, device_id"
    ).fetchall()
    for i, (did,) in enumerate(devs):
        name = "本机(演示)" if i == 0 else f"演示设备 {chr(64 + i)}"
        cur.execute("UPDATE devices SET display_name = ? WHERE device_id = ?", (name, did))

    # AI 派生物:图描述/嵌入/去重映射基于真实截图,删;段总结文本换语料
    for table in ("ai_image_descriptions", "screenshot_embeddings", "screenshot_dedup_map"):
        try:
            cur.execute(f"DELETE FROM {table}")
        except sqlite3.OperationalError:
            pass  # 老库可能没有该表
    seg = cur.execute(
        "SELECT rowid, segment_idx FROM ai_summaries WHERE content != ''"
    ).fetchall()
    cur.executemany(
        "UPDATE ai_summaries SET content = ?, error = NULL WHERE rowid = ?",
        [(SEGMENT_SUMMARIES[idx % len(SEGMENT_SUMMARIES)], rid) for rid, idx in seg],
    )

    # 设置 JSON:关采集/同步,清一切像凭据的字段,截图路径指向演示目录
    row = cur.execute("SELECT data FROM settings_store WHERE id = 1").fetchone()
    if row:
        data = json.loads(row[0])
        cur.execute(
            "UPDATE settings_store SET data = ? WHERE id = 1",
            (json.dumps(scrub_settings(data, demo_root), ensure_ascii=False),),
        )

    conn.commit()
    conn.close()
    return originals


SENSITIVE_KEY_HINTS = ("key", "token", "secret", "endpoint", "client_id", "clientid")
FORCE_FALSE = {
    "capture_enabled",
    "captureEnabled",
    "screenshot_enabled",
    "screenshotEnabled",
    "auto_start",
    "autoStart",
    "memory_ocr_resident",
    "memoryOcrResident",
    "external_enabled",
    "externalEnabled",
}


def scrub_settings(node, demo_root):
    if isinstance(node, dict):
        out = {}
        for k, v in node.items():
            lk = k.lower()
            if k in FORCE_FALSE:
                out[k] = False
            elif lk in ("screenshot_path", "screenshotpath"):
                out[k] = str(demo_root / "screenshots")
            elif isinstance(v, str) and any(h in lk for h in SENSITIVE_KEY_HINTS):
                out[k] = ""
            else:
                out[k] = scrub_settings(v, demo_root)
        return out
    if isinstance(node, list):
        return [scrub_settings(x, demo_root) for x in node]
    return node


# ───────────────────────── 净化:记忆库 ─────────────────────────


def sanitize_memory(db):
    conn = sqlite3.connect(db)
    cur = conn.cursor()

    # frames:标题→语料(path 只是文件名,无内容;对应文件在演示目录不存在)
    rows = cur.execute(
        "SELECT path, app_id, title FROM frames WHERE title IS NOT NULL"
    ).fetchall()
    cur.executemany(
        "UPDATE frames SET title = ? WHERE path = ?",
        [(fake_title(a or "", t), p) for p, a, t in rows],
    )

    # 文本会话:标题+正文→语料(UPDATE 触发器自动维护 FTS);行级留痕按新正文重建
    sessions = cur.execute("SELECT id, app_id, title FROM text_sessions").fetchall()
    first = {
        sid: (fp, ts)
        for sid, fp, ts in cur.execute(
            "SELECT session_id, MIN(first_path), MIN(first_ts) "
            "FROM session_lines GROUP BY session_id"
        )
    }
    new_lines = []
    for sid, app_id, title in sessions:
        n = 2 + int(hashlib.md5(str(sid).encode()).hexdigest(), 16) % 3  # 2-4 段
        paras = list(dict.fromkeys(pick(OCR_PARAGRAPHS, f"{sid}:{i}") for i in range(n)))
        cur.execute(
            "UPDATE text_sessions SET title = ?, text = ? WHERE id = ?",
            (fake_title(app_id or "", title or ""), "\n".join(paras), sid),
        )
        fp, ts = first.get(sid, (None, None))
        if fp:
            new_lines += [(sid, i, para, fp, ts) for i, para in enumerate(paras)]
    cur.execute("DELETE FROM session_lines")
    cur.executemany(
        "INSERT INTO session_lines(session_id, line_no, text, first_path, first_ts) "
        "VALUES (?, ?, ?, ?, ?)",
        new_lines,
    )


    conn.commit()
    conn.close()


# ───────────────────────── 今日增密 ─────────────────────────


def densify_today(main_db):
    """把演示库里最忙一天的活动节奏整体搬到今天(截到当前时刻),
    日报同步搬——现场演示时"今天"不再是残缺的半天。"""
    from datetime import datetime, date, timedelta

    conn = sqlite3.connect(main_db)
    cur = conn.cursor()
    today = date.today().isoformat()
    src = cur.execute(
        "SELECT local_date FROM activities WHERE local_date != ? "
        "GROUP BY local_date ORDER BY SUM(duration_secs) DESC LIMIT 1",
        (today,),
    ).fetchone()
    if not src:
        conn.close()
        return
    src_date = src[0]
    delta = date.fromisoformat(today) - date.fromisoformat(src_date)
    now = datetime.now().astimezone()

    cur.execute("DELETE FROM activities WHERE local_date = ?", (today,))
    rows = cur.execute(
        "SELECT started_at, ended_at, local_hour, process_name, window_title, "
        "category_id, device_id, origin, updated_at FROM activities "
        "WHERE local_date = ? ORDER BY started_at",
        (src_date,),
    ).fetchall()
    inserted = 0
    for sa, ea, lh, proc, title, cat, dev, origin, upd in rows:
        try:
            ns = datetime.fromisoformat(sa) + delta
            ne = datetime.fromisoformat(ea) + delta
        except ValueError:
            continue
        if ns >= now:
            continue
        ne = min(ne, now)
        dur = int((ne - ns).total_seconds())
        if dur <= 0:
            continue
        cur.execute(
            "INSERT INTO activities(started_at, ended_at, duration_secs, local_date, "
            "local_hour, process_name, window_title, category_id, screenshot_path, "
            "image_hash, device_id, remote_id, updated_at, origin) "
            "VALUES (?, ?, ?, ?, ?, ?, ?, ?, NULL, NULL, ?, NULL, ?, ?)",
            (ns.isoformat(), ne.isoformat(), dur, today, lh, proc, title, cat, dev, upd, origin),
        )
        inserted += 1

    # 日报:今天的段总结,独立挑"最近一个有日报的日子"当模板
    # (活动最忙的那天不一定生成过日报)
    cur.execute("DELETE FROM ai_summaries WHERE local_date = ?", (today,))
    src2 = cur.execute(
        "SELECT local_date FROM ai_summaries WHERE local_date != ? "
        "ORDER BY local_date DESC LIMIT 1",
        (today,),
    ).fetchone()
    if src2:
        cols = [r[1] for r in cur.execute("PRAGMA table_info(ai_summaries)")]
        others = [c for c in cols if c != "local_date"]
        cur.execute(
            f"INSERT INTO ai_summaries(local_date, {', '.join(others)}) "
            f"SELECT ?, {', '.join(others)} FROM ai_summaries WHERE local_date = ?",
            (today, src2[0]),
        )
    conn.commit()
    conn.close()
    print(f"[demo] 今日增密:以 {src_date} 为模板生成 {inserted} 条活动(截至当前时刻)。")


def densify_today_memory(mem_db):
    """记忆库同步增密:把最忙一天的文本会话搬到今天(搜索/时间线证据今天也有货)。"""
    from datetime import date, datetime, timedelta

    conn = sqlite3.connect(mem_db)
    cur = conn.cursor()
    today = date.today().isoformat()
    src = cur.execute(
        "SELECT local_date FROM text_sessions WHERE local_date != ? "
        "GROUP BY local_date ORDER BY COUNT(*) DESC LIMIT 1",
        (today,),
    ).fetchone()
    if not src:
        conn.close()
        return
    src_date = src[0]
    delta = date.fromisoformat(today) - date.fromisoformat(src_date)
    now = datetime.now().astimezone()

    old_ids = [r[0] for r in cur.execute(
        "SELECT id FROM text_sessions WHERE local_date = ?", (today,)
    )]
    cur.executemany("DELETE FROM session_lines WHERE session_id = ?", [(i,) for i in old_ids])
    cur.execute("DELETE FROM text_sessions WHERE local_date = ?", (today,))

    rows = cur.execute(
        "SELECT id, started_ts, ended_ts, app_id, title, text, origin_device "
        "FROM text_sessions WHERE local_date = ? ORDER BY started_ts",
        (src_date,),
    ).fetchall()
    copied = 0
    for sid, sts, ets, app_id, title, text, odev in rows:
        try:
            ns = datetime.fromisoformat(sts) + delta
            ne = datetime.fromisoformat(ets) + delta
        except ValueError:
            continue
        if ns >= now:
            continue
        ne = min(ne, now)
        cur.execute(
            "INSERT INTO text_sessions(local_date, started_ts, ended_ts, app_id, title, "
            "text, guid, origin_device) VALUES (?, ?, ?, ?, ?, ?, NULL, ?)",
            (today, ns.isoformat(), ne.isoformat(), app_id, title, text, odev),
        )
        new_id = cur.lastrowid
        cur.execute(
            "INSERT INTO session_lines(session_id, line_no, text, first_path, first_ts) "
            "SELECT ?, line_no, text, first_path, first_ts FROM session_lines "
            "WHERE session_id = ?",
            (new_id, sid),
        )
        copied += 1
    conn.commit()
    conn.close()
    print(f"[demo] 记忆增密:今天复制 {copied} 条文本会话(模板 {src_date})。")


def seed_chat(mem_db, main_db):
    """预置两组演示问答,数字从演示库实算——录屏时点开就是对的数。"""
    from datetime import datetime, timedelta

    c = sqlite3.connect(main_db)
    today_top = c.execute(
        "SELECT process_name, ROUND(SUM(duration_secs)/3600.0,1) h FROM activities "
        "WHERE local_date = date('now','localtime') GROUP BY process_name "
        "ORDER BY h DESC LIMIT 3"
    ).fetchall()
    today_total = c.execute(
        "SELECT ROUND(SUM(duration_secs)/3600.0,1) FROM activities "
        "WHERE local_date = date('now','localtime')"
    ).fetchone()[0] or 0
    bili = c.execute(
        "SELECT started_at, duration_secs FROM activities "
        "WHERE window_title LIKE '%哔哩哔哩%' "
        "AND local_date >= date('now','localtime','start of month') ORDER BY started_at"
    ).fetchall()
    c.close()

    bili_hours = round(sum(d for _, d in bili) / 3600.0, 1)
    # 「看了几次」= 按 30 分钟断流分组的观看会话数
    times, last_end = 0, None
    for sa, dur in bili:
        try:
            st = datetime.fromisoformat(sa)
        except ValueError:
            continue
        if last_end is None or (st - last_end) > timedelta(minutes=30):
            times += 1
        last_end = st + timedelta(seconds=dur)

    convs = []
    if today_top:
        tops = ";".join(f"{i+1}. {n}({h} 小时)" for i, (n, h) in enumerate(today_top))
        convs.append((
            "今日应用使用统计",
            "我今天用得最多的三个应用是什么?各用了多久?",
            f"今天到目前为止你共使用电脑约 {today_total} 小时,前三名是:{tops}。"
            f"其中 {today_top[0][0]} 占全天约 "
            f"{round(today_top[0][1] / today_total * 100) if today_total else 0}%。",
        ))
    if bili:
        convs.append((
            "B 站观看统计",
            "这个月我在 B 站看了多久?大概看了多少次?",
            f"这个月你在 B 站累计观看约 {bili_hours} 小时,分 {times} 次看完,"
            f"基本集中在晚上 7 点到 11 点之间;看得最多的是技术类和音乐类视频。",
        ))

    m = sqlite3.connect(mem_db)
    cur = m.cursor()
    cur.execute("DELETE FROM chat_messages")
    cur.execute("DELETE FROM chat_conversations")
    conv_cols = [r[1] for r in cur.execute("PRAGMA table_info(chat_conversations)")]
    msg_cols = [r[1] for r in cur.execute("PRAGMA table_info(chat_messages)")]
    ts0 = cur.execute(
        "SELECT COALESCE(MAX(ended_ts), '2026-07-07T09:00:00Z') FROM text_sessions"
    ).fetchone()[0]
    for idx, (title, q, a) in enumerate(convs, start=1):
        cv = {"id": idx, "title": title, "created_ts": ts0, "updated_ts": ts0}
        cols = [x for x in conv_cols if x in cv]
        cur.execute(
            f"INSERT INTO chat_conversations({', '.join(cols)}) "
            f"VALUES ({', '.join('?' * len(cols))})",
            [cv[x] for x in cols],
        )
        for role, content in (("user", q), ("assistant", a)):
            v = {"conversation_id": idx, "role": role, "content": content,
                 "citations": None, "degraded": 0, "created_ts": ts0}
            cols = [x for x in msg_cols if x in v]
            cur.execute(
                f"INSERT INTO chat_messages({', '.join(cols)}) "
                f"VALUES ({', '.join('?' * len(cols))})",
                [v[x] for x in cols],
            )
    m.commit()
    m.close()
    print(f"[demo] 预置问答:{len(convs)} 组(数字实算:B站 {bili_hours}h/{times} 次)。")


# ───────────────────────── 自查与启动 ─────────────────────────


def verify(main_db, originals):
    """隐私自查:抽样的原始标题一个都不许在演示库里出现。"""
    conn = sqlite3.connect(f"file:{main_db}?mode=ro", uri=True)
    leaked = [
        t
        for t in originals
        if conn.execute(
            "SELECT 1 FROM activities WHERE window_title = ? LIMIT 1", (t,)
        ).fetchone()
    ]
    conn.close()
    if leaked:
        sys.exit(f"[demo] 隐私自查失败:{len(leaked)} 条原始标题仍在演示库中,已中止。")
    print(f"[demo] 隐私自查通过:抽样 {len(originals)} 条原始标题,演示库零残留。")


def installed_app_cmd():
    """已安装正式版 app 的启动命令(默认演示入口;不占用 npm run dev)。"""
    if platform.system() == "Darwin":
        for base in ("/Applications", str(Path.home() / "Applications")):
            p = Path(base) / "Hindsight.app" / "Contents" / "MacOS" / "hindsight"
            if p.is_file():
                return [str(p)]
        return None
    if platform.system() == "Windows":
        for base in (os.environ.get("LOCALAPPDATA", ""), os.environ.get("PROGRAMFILES", "")):
            p = Path(base) / "Hindsight" / "hindsight.exe"
            if base and p.is_file():
                return [str(p)]
        return None
    return None


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--fresh", action="store_true", help="删除旧演示数据重新生成")
    ap.add_argument("--gen-only", action="store_true", help="只生成,不启动 app")
    ap.add_argument("--dev", action="store_true", help="用 tauri dev 启动(默认用已安装正式版)")
    args = ap.parse_args()

    real_root = data_root()
    uid = active_uid()
    main_name, mem_name = db_names(uid)
    real_main = real_root / main_name
    if not real_main.is_file():
        sys.exit(f"[demo] 找不到真实主库:{real_main}")

    demo_root = real_root / "demo"
    if args.fresh and demo_root.exists():
        shutil.rmtree(demo_root)
        print(f"[demo] 已删除旧演示数据:{demo_root}")

    demo_main = demo_root / main_name
    if demo_main.is_file():
        print(f"[demo] 演示数据已存在(--fresh 可重建):{demo_root}")
    else:
        print(f"[demo] 克隆(只读)→ 净化:{real_root} → {demo_root}")
        clone_ro(real_main, demo_main)
        originals = sanitize_main(demo_main, demo_root)

        real_mem = real_root / mem_name
        if real_mem.is_file():
            demo_mem = demo_root / mem_name
            clone_ro(real_mem, demo_mem)
            sanitize_memory(demo_mem)

        (demo_root / "screenshots").mkdir(parents=True, exist_ok=True)
        # AI 引擎/模型无隐私,软链共享,演示实例免重新下载(仅类 Unix)
        real_ai = real_root / "ai"
        demo_ai = demo_root / "ai"
        if real_ai.is_dir() and not demo_ai.exists() and os.name == "posix":
            os.symlink(real_ai, demo_ai)

        densify_today(demo_main)
        if (demo_root / mem_name).is_file():
            densify_today_memory(demo_root / mem_name)
            seed_chat(demo_root / mem_name, demo_main)

        verify(demo_main, originals)
        print("[demo] 生成完成。")

    if args.gen_only:
        print(f"[demo] 手动启动:HINDSIGHT_DATA_DIR='{demo_root}' <app 或 npm run tauri dev>")
        return

    env = os.environ.copy()
    env["HINDSIGHT_DATA_DIR"] = str(demo_root)
    print("[demo] 提示:app 是单实例,请先从托盘退出正在运行的 Hindsight。")
    if args.dev:
        cmd = ["npm", "run", "tauri", "dev"]
    else:
        cmd = installed_app_cmd()
        if not cmd:
            sys.exit("[demo] 未找到已安装的 Hindsight;可用 --dev 走开发构建,或 --gen-only 后手动启动。")
    print(f"[demo] 以演示数据目录启动:{demo_root}")
    subprocess.run(cmd, env=env, check=False)


if __name__ == "__main__":
    main()
