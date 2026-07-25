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
import re
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
    "chat": [
        "Aurora 项目组", "产品评审群", "运维值班群", "周报提醒", "前端小分队",
        "数据管线告警", "李维(设计)", "陈晨", "王一帆", "赵樱", "林小满",
        "苏合(PM)", "老许", "招聘协调群", "Aurora 发布窗口", "摸鱼茶话会",
        "客服工单同步", "家庭群", "羽毛球周三局", "楼下拼咖啡",
    ],
    "mail": [
        "收件箱 - aurora.dev 邮箱", "Q3 目标评审会 - 日历邀请",
        "【账单】云服务 7 月用量提醒", "Re: 管线延迟 P95 报警阈值",
        "GitHub 通知摘要", "Aurora 周报 #29", "转发:新办公室工位图",
        "【物流】您的包裹已发货", "发票开具确认", "Re: Re: 压测窗口协调",
        "安全提醒:新设备登录", "内推简历:高级后端工程师",
    ],
    "doc": [
        "Aurora 架构设计.md", "季度复盘.docx", "会议纪要 2026-07", "本周待办",
        "读书笔记 — 《数据密集型应用》", "面试题库整理", "旅行清单",
        "管线容量规划草稿", "新人入职指引 v3", "技术分享:WAL 与检查点",
        "OKR 对齐表", "灰度发布 checklist", "踩坑记录:时区与夏令时",
        "隐私合规自查清单", "读书笔记 — 《运维之道》", "年度设备采购计划",
    ],
    "media": [
        "星际旅人 4K — 播放器", "纪录片:深海 — 第 2 集", "Lo-fi 工作歌单",
        "白噪音 — 雨声", "「星野」新专辑", "City Pop 精选集",
        "纪录片:代码改变世界 — 第 1 集", "钢琴练习曲集", "播客:开发者茶话 EP.42",
        "演唱会 Live 合集", "冥想引导 — 10 分钟", "电影原声带精选",
    ],
    "terminal": [
        "cargo build --release", "npm run tauri dev", "git rebase -i HEAD~3",
        "htop", "ssh aurora-staging", "tail -f service.log", "docker compose up",
        "kubectl get pods -n aurora", "pytest -k pipeline", "git bisect run",
        "rsync 备份脚本", "psql aurora_prod", "cargo clippy --all-targets",
        "vim deploy.yaml", "brew upgrade", "just release",
    ],
    "design": [
        "Aurora 控制台改版稿", "组件库 v2", "图标集整理", "海报草稿 0713",
        "Logo 微调", "官网首页 hero 图", "移动端适配标注", "配色方案 A/B",
        "字体对比板", "成片粗剪 0726", "口播字幕对轨", "封面图打样",
    ],
    "ai": [
        "Rust 生命周期报错求解 - Claude", "SQL 窗口函数写法 - ChatGPT",
        "重构方案对比 - Claude", "正则表达式调试 - ChatGPT",
        "英文邮件润色 - Claude", "K8s 探针配置 - ChatGPT",
        "面试自我介绍打磨 - Claude", "旅行行程规划 - ChatGPT",
        "SQLite 锁争用分析 - Claude", "周报要点提炼 - ChatGPT",
    ],
    "sheets": [
        "管线容量测算.xlsx", "Q3 预算表.xlsx", "值班排班表.xlsx",
        "招聘漏斗统计.xlsx", "对账单 2026-06.xlsx", "回放耗时记录.xlsx",
    ],
    "slides": [
        "Aurora 里程碑.pptx", "技术分享:向量检索入门.pptx", "团队季度汇报.pptx",
        "灰度发布方案评审.pptx", "新人培训:数据管线概览.pptx",
    ],
    "pdf": [
        "SQLite 官方文档(打印版).pdf", "Designing Data-Intensive Applications.pdf",
        "劳动合同模板.pdf", "路由器说明书.pdf", "论文:LSM-Tree 优化综述.pdf",
        "报销单扫描件.pdf", "签证材料清单.pdf", "显示器色彩校准指南.pdf",
    ],
    "game": [
        "Steam", "星露谷物语", "文明 VI", "糖豆人", "Epic Games Launcher", "棋弈对战",
    ],
    "self": ["Hindsight"],
    "default": [
        "Aurora 项目资料", "本周待办", "收件箱", "设置", "快速笔记",
        "文件整理", "下载内容", "系统偏好",
    ],
}

# 组合式语料:主题 × 站点/结构模板,映射空间几百~上千,
# 避免"全天几百个真实标题被压进 10 个假值"的循环重复感。
# 浏览器标题按「题材家族」组合:每个家族配语义匹配的站点池,
# 避免"镰仓一日游 — 技术周刊"这类乱配;家族列表带权重(技术类出现更多)。
BROWSER_FAMILIES = [
    # (题材池, 站点池) —— 技术,权重 ×3
    (
        [
            "Rust 异步运行时对比", "SQLite WAL 模式详解", "PostgreSQL 分区表实战",
            "Tauri 2.0 发布说明", "一致性哈希图解", "Raft 论文精读笔记",
            "开源许可证对比", "CRDT 入门", "向量数据库选型",
            "K8s 探针最佳实践", "Nginx 限流配置", "TLS 证书自动续期",
            "SQLite 全文检索实践", "Rust 错误处理模式", "WebSocket 断线重连设计",
            "浏览器渲染管线科普",
        ],
        [" — 技术周刊", " — 开发者博客", " - 知乎", " — 掘金",
         " - Stack Overflow", " - Google 搜索", " - V2EX"],
    ),
    (
        [
            "机械键盘选购指南", "站立办公桌横评", "降噪耳机怎么选",
            "人体工学椅对比", "显示器支架推荐", "NAS 硬盘怎么选",
        ],
        [" — 值得买", " - 知乎", " - Google 搜索", " — 少数派"],
    ),
    (
        [
            "东京五日游攻略", "镰仓一日游路线", "京都红叶季时间表",
            "冲绳自驾注意事项", "大阪美食地图", "富士山周边住宿",
        ],
        [" - 知乎", " - Google 搜索", " — 穷游锦囊"],
    ),
    (
        [
            "健身房新手计划", "跑步心率区间科普", "咖啡手冲参数表",
            "程序员颈椎保养", "居家办公效率清单", "个税专项扣除说明",
            "指数基金定投入门", "租房合同避坑指南", "《数据密集型应用》书评",
            "Figma 组件库最佳实践", "配色理论入门", "字体排印基础",
        ],
        [" - 知乎", " — 少数派", " - Google 搜索"],
    ),
]
# 家族抽签权重:技术 ×2、生活 ×2,数码/旅行 ×1(偏日常,大家都看得懂)
_BROWSER_DECK = [0, 0, 1, 2, 3, 3]


# B 站产出按真实库校准(真实约 11.6% 行 / 14.8% 时长 / 113 次每月)。
# 两级门控:先抽"B 站时段"(某天某小时,BILI_HOUR_PCT),命中时段内的
# 浏览行大多连片变 B 站(BILI_ROW_PCT)——观看是成段的,不是零星插针,
# 会话次数与单次时长才像真人。无时段上下文时退回低概率兜底。
BILI_HOUR_PCT = 40
BILI_ROW_PCT = 47
BILI_FALLBACK_PCT = 10


def _browser_title(key, ctx=None):
    h = hashlib.md5(key.encode("utf-8")).hexdigest()
    if ctx is not None:
        d, lh = ctx
        hour_roll = int(hashlib.md5(f"bh|{d}|{lh}".encode("utf-8")).hexdigest()[:6], 16) % 100
        hit = hour_roll < BILI_HOUR_PCT and int(h[20:26], 16) % 100 < BILI_ROW_PCT
    else:
        hit = int(h[20:26], 16) % 100 < BILI_FALLBACK_PCT
    if hit:
        return pick(BILI_TITLES, key)
    topics, sites = BROWSER_FAMILIES[_BROWSER_DECK[int(h[:4], 16) % len(_BROWSER_DECK)]]
    return topics[int(h[4:12], 16) % len(topics)] + sites[int(h[12:20], 16) % len(sites)]


CODE_FILES = [
    "main.rs", "pipeline.py", "invoice.ts", "Dashboard.tsx", "queue.go",
    "schema.sql", "deploy.sh", "theme.css", "worker.rs", "api.ts",
    "ingest.py", "config.toml", "auth.go", "Chart.tsx", "migrate.sql",
    "Dockerfile", "utils.rs", "hooks.ts", "consumer.py", "README.md",
    "cache.go", "types.d.ts", "backfill.py", "router.rs",
]
CODE_PROJECTS = [
    "Aurora 数据管线", "Aurora 控制台", "billing-service",
    "notify-worker", "infra-scripts", "aurora-sdk",
]


def _pick2(key, pool_a, pool_b, joiner="{a}{b}"):
    h = hashlib.md5(key.encode("utf-8")).hexdigest()
    a = pool_a[int(h[:8], 16) % len(pool_a)]
    b = pool_b[int(h[8:16], 16) % len(pool_b)]
    return joiner.format(a=a, b=b)


TITLE_GEN = {
    "browser": _browser_title,  # 唯一收 ctx 的生成器
    "code": lambda key: _pick2(key, CODE_FILES, CODE_PROJECTS, "{a} — {b}"),
    # 游戏窗口标题现实中就是游戏名:直接用进程名(进程名本就保留,去掉 .exe 尾巴),
    # 避免游戏进程顶着"收件箱"之类乱入标题
    "game": lambda key: re.sub(r"\.(exe|app)$", "", key.split("|", 1)[0], flags=re.I),
}

BUCKETS = [
    ("code", ["cursor", "code", "idea", "zed", "studio", "vim", "sublime"]),
    ("browser", ["chrome", "safari", "edge", "firefox", "arc", "brave", "browser"]),
    ("chat", ["wechat", "weixin", "微信", "slack", "discord", "telegram", "qq", "teams", "lark", "飞书", "dingtalk", "钉钉"]),
    ("mail", ["mail", "outlook", "thunderbird", "spark", "airmail"]),
    ("doc", ["word", "pages", "notion", "obsidian", "typora", "onenote", "docs", "note"]),
    ("sheets", ["excel", "numbers", "wps表格"]),
    ("slides", ["powerpoint", "keynote", "wps演示"]),
    ("design", ["figma", "sketch", "photoshop", "illustrator", "blender", "procreate", "affinity", "capcut", "剪映", "davinci"]),
    ("ai", ["chatgpt", "claude", "copilot", "gemini", "poe"]),
    ("pdf", ["preview", "acrobat", "skim", "pdf"]),
    ("media", ["music", "spotify", "iina", "vlc", "quicktime", "player", "netease", "podcast"]),
    ("game", ["steam", "epic", "battle.net", "riot", "star rail", "honkai", "genshin", "原神", "崩坏", "wuthering", "league", "cs2", "dota"]),
    ("terminal", ["terminal", "iterm", "warp", "alacritty", "kitty", "powershell", "cmd", "ターミナル", "终端"]),
    ("self", ["hindsight"]),
]

# 签名段落:只种进极少数会话(~千分之一)。搜索演示搜"Keychron"时
# 命中个位数才像"看过一次订单页",而不是通用背景语料那样命中几千条。
OCR_RARE = [
    "订单确认:Keychron K8 机械键盘(茶轴),订单号 AUR-2026-0713,"
    "实付 ¥399.00,预计 7 月 15 日送达。收货后记得先试打半小时再决定要不要换轴。",
    "体检预约确认:7 月 30 日上午 8:30,记得前一晚十点后禁食;"
    "带身份证,常规项目约两小时出报告。",
]

OCR_PARAGRAPHS = [
    "会议纪要:确认季度汇报定在下月 5 日,PPT 由我负责初稿;"
    "预算部分等财务的数出来再补;下次对进度是 7 月 18 日。",
    "东京行程草稿:D1 浅草寺-晴空塔,D2 三鹰之森吉卜力(记得提前一个月抢票),"
    "D3 镰仓一日游;酒店定在上野附近,方便坐京成线。",
    "待办清单:1) 汇报 PPT 补最后两页;2) 给合作方回邮件确认交期;"
    "3) 订周三羽毛球场;4) 给路由器换 DNS;5) 报销单截止周五。",
    "报销提醒:6 月差旅共 3 笔待提交,截止 7 月 25 日;"
    "打车发票记得合并成一个 PDF 上传,住宿发票要附行程单。",
    "健身记录:本周三练——深蹲 5×5 @70kg,硬拉 3×5 @90kg,"
    "引体 4×8;下周深蹲加 2.5kg,注意腰带位置。",
    "快递通知:您的包裹已到菜鸟驿站(编号 8-3-2107),请凭取件码领取;"
    "另一件从上海发出的包裹预计明天送达。",
    "租房备忘:看了两套,都是两室一厅;A 套近地铁但朝北,B 套朝南带阳台"
    "但要多走十分钟;和中介约了周六再看一次 B 套,记得确认物业费。",
    "菜谱收藏:番茄炖牛腩——牛腩焯水后小火炖 40 分钟再下番茄,"
    "最后十分钟放土豆;盐要最后放,不然肉柴。",
    "读书摘录:把大目标拆成够小的下一步,行动的阻力会小很多;"
    "计划的意义不在于被严格执行,而在于让你知道偏航了多少。",
    "工作文档:项目时间线更新——需求确认本周五截止,"
    "下周进入制作期,预留三天给审校;风险项是素材到位时间。",
    "群公告:本周五团建改为聚餐,地点在公司附近的川菜馆,"
    "六点半集合;有忌口的同学在接龙里备注一下。",
    "课程笔记:摄影构图第 3 课——三分法之外,试着用引导线和框架式构图;"
    "作业是本周拍三张带前景的照片。",
    "客服回复:您反馈的导出文件乱码问题已确认为老版本缺陷,"
    "升级到最新版即可解决;给您带来不便非常抱歉。",
    "周末计划:周六上午骑车去湿地公园,中午在附近吃面;"
    "周日在家大扫除加换季收纳,晚上把下周的衬衫熨出来。",
]

# 时段总结按 segment_idx(0 深夜 / 1 早上 / 2 上午 / 3 下午 / 4 晚上)分组,
# 组内多变体按行哈希轮换——避免"深夜时段配上『上午…』"的文不对题。
SEGMENT_SUMMARIES = {
    0: [
        "深夜基本没碰电脑。睡前扫了一眼群消息,没有需要马上回的;把手机上没看完的一篇文章加进了稍后读,又确认了明早的闹钟和日程,十二点前就休息了。相比上周偶尔的熬夜,这周睡前不看工作消息的习惯保持得不错,深夜的屏幕时间明显在收缩。",
        "追完一集纪录片才睡。看完顺手把桌面清了清:散落的截图归进文件夹,两个用完的安装包删掉,下载目录清爽了不少;又把明天要带的文件拖进了随身盘。之后再没有屏幕活动,整段处于休息状态,入睡时间比昨天早了约半小时。",
    ],
    1: [
        "早上以收拾和启动为主。先过了一遍邮箱,广告和通知直接归档,两封需要确认的邮件当场回掉:一封确认了周四的拜访时间,另一封把资料补发了过去。随后把今天的待办列进备忘,按优先级排了序,顺手确认了上午会议的时间和会议室;正式坐下干活前还刷了十分钟新闻,九点前进入状态。",
        "起来先看了昨晚攒下的群消息,逐条回复了几处约时间的;浏览器里读了两篇公众号文章,一篇讲衣柜换季收纳,一篇讲久坐族的颈椎放松动作,都收藏进了稍后整理的夹子。之后把桌面和下载目录清了清,九点后切入正式工作,节奏平稳没有匆忙感。",
    ],
    2: [
        "上午的大头是季度汇报的 PPT。先把框架和目录定下来,再逐页填数据和配图:上半年的两张对比图重画了一遍,配色统一成公司模板的蓝灰系;中途让同事帮忙核对了两处数字,改掉一处口径不一致。临近午间开了个二十分钟的短会对进度,会后把改动点直接落进演示稿。整体专注度不错,群消息只零星回了几条,PPT 完成度到了七成。",
        "上午在文档和表格之间来回。先把上周的总结文档改完发出去,补上了两条遗漏的进展;又打开报表核对这个月的数据,发现两个格子填错了行,顺着公式改了回来,和上月的合计对上了。中间抽空回复了几条工作群的讨论,给下午的会议订好了会议室,还把要用的材料提前发给了参会的同事。",
        "上午一半时间在改自己手头的小工具:一个反复出现的小毛病终于定位到了,是两处设置互相顶掉对方,改好后试了几轮没再复现,顺手把界面上两个不顺眼的间距也调了。另一半时间整理共享文档里的反馈意见,逐条回复标记,过期的附件清理掉;临近午饭把改动记录写进了更新说明。节奏平稳,没有被会议打断。",
    ],
    3: [
        "下午两点开了一小时的例会,确认了这期的分工和时间节点,自己领了汇报材料和两页数据附录。会后把会议纪要整理发到群里,@了两位需要跟进的同事;接着继续改 PPT,把图表的配色和字号统一了一遍,标题层级重新理顺。临下班前把明天要用的材料打包发给同事,收件箱清到只剩两封待回。",
        "午后先集中处理文档:合作方发来的修订稿逐条过完,能接受的直接接受,有疑问的加了批注等对方确认,前后花了一个多小时。四点后切到浏览器查资料,把下周出差的酒店比了三家,选了离客户近、含早餐的那家订下,又顺手把高铁票改签到早一班。下班前回了一圈消息,今天的事没有拖到明天的。",
        "下午被两个会切开:前一个聊下期方案,确定了先做样稿再放量的路线;后一个对预算,砍掉了两项优先级不高的开支。会议间隙把演示稿里遗留的批注消完,又把上个月的报销单补交了,发票合并成一个 PDF 传了上去。会议偏多,但每件事都有收尾,下班时桌面和收件箱都是干净的。",
    ],
    4: [
        "晚间节奏放缓。先看了两集剧放松,片尾顺手在购物网站比了比机械键盘的价格,茶轴和红轴的评价各看了几条,加进购物车没急着下单,打算等周末的活动价。睡前把明天的待办列了出来,清了下下载文件夹,把这周拍的照片挑了几张传进相册;十一点半前合上电脑。",
        "晚上刷了会儿 B 站,看完一个数码测评和一个旅行 vlog,又顺着推荐看了半集美食纪录片。之后把周末的出行路线在地图上标好:上午的公园、中午的面馆、下午的书店连成一条线,查了天气说周六多云适合骑车。十一点前合上电脑休息,屏幕时间比昨晚短。",
        "晚上先打了两把游戏休整,赢一把输一把,心态平稳退出。随后把白天没看完的文档翻完,给两处数据标了疑问准备明天当面问;又把周四要交的材料检查了一遍格式,目录页码都对上了。睡前刷了刷手机,把闹钟往后调了半小时——明天不用赶早。",
    ],
}

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
    "自建 NAS 从入门到吃灰_哔哩哔哩_bilibili",
    "【纪录片】硅谷车库往事 P1_哔哩哔哩_bilibili",
    "30 天早起挑战真实记录_哔哩哔哩_bilibili",
    "这把人体工学椅值不值 1500?_哔哩哔哩_bilibili",
]
HINDSIGHT_TITLES = [
    "Hindsight — capture/service.rs",
    "Hindsight — memory/digest.rs",
    "Hindsight — ChatView.tsx",
    "Hindsight — screen-memory.md",
    "Hindsight — ai/ocr.rs",
    "Hindsight — SettingsPage.tsx",
    "Hindsight — chat/tools.rs",
    "Hindsight — SearchPage.tsx",
    "Hindsight — repo/reports.rs",
    "Hindsight — sync/engine/pull.rs",
]

# ───────────────────────── 工具 ─────────────────────────


def parse_ts(value):
    """解析库里的 RFC3339 时间戳:秒的小数部分裁到 6 位。
    (Rust 侧写入纳秒精度,老版本 Python 的 fromisoformat 会直接拒收。)"""
    from datetime import datetime

    m = re.match(r"^(.*?\.)(\d{7,})([+-].*|Z?)$", value)
    if m:
        value = f"{m.group(1)}{m.group(2)[:6]}{m.group(3)}"
    return datetime.fromisoformat(value)


def pick(pool, key):
    h = int(hashlib.md5(key.encode("utf-8")).hexdigest(), 16)
    return pool[h % len(pool)]


def bucket_of(process_name):
    p = (process_name or "").lower()
    for name, kws in BUCKETS:
        if any(k in p for k in kws):
            return name
    return "default"


def fake_title(process_name, original, ctx=None):
    """ctx=(local_date, local_hour):给浏览器桶做 B 站时段聚簇用,其余桶忽略。"""
    b = bucket_of(process_name)
    key = f"{process_name}|{original}"
    if b == "browser":
        return _browser_title(key, ctx)
    if b in TITLE_GEN:
        return TITLE_GEN[b](key)
    return pick(TITLES[b], key)


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
        "SELECT id, process_name, window_title, local_date, local_hour "
        "FROM activities WHERE window_title IS NOT NULL"
    ).fetchall()
    cur.executemany(
        "UPDATE activities SET window_title = ? WHERE id = ?",
        [(fake_title(p, t, (d, lh)), i) for i, p, t, d, lh in rows],
    )
    cur.execute("UPDATE activities SET screenshot_path = NULL, image_hash = NULL")

    # 注入演示素材:代码类的 2/3 → Hindsight 开发("开发花了多久"有货可查)。
    # B 站标题不再强灌,由 _browser_title 的门控按真实占比自然产出。
    rows = cur.execute(
        "SELECT id, process_name FROM activities WHERE window_title IS NOT NULL"
    ).fetchall()
    hs = []
    for i, proc in rows:
        if bucket_of(proc) == "code" and i % 3 != 2:
            hs.append((pick(HINDSIGHT_TITLES, f"h{i}"), i))
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
        [
            (pick(SEGMENT_SUMMARIES.get(idx, SEGMENT_SUMMARIES[3]), f"seg{rid}"), rid)
            for rid, idx in seg
        ],
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
        if int(hashlib.md5(f"rare|{sid}".encode("utf-8")).hexdigest()[:8], 16) % 1200 == 0:
            paras.append(pick(OCR_RARE, f"rare|{sid}"))
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
    """今天固定合成为"标准 8 小时工作日"(09:00-12:30 + 13:30-18:30):
    取工作类时长最高的历史日(单设备)的活动序列,保留每条时长与切换节奏、
    长空档压成 3 分钟,重排时间戳搬到今天。不按当前时刻截断——
    无论何时录屏,"今天"都是一个完整、干净的工作日。"""
    from datetime import datetime, date, time, timedelta

    WORK_APPS = (
        "Code", "Visual Studio Code", "Cursor", "Typora", "Microsoft Word",
        "Microsoft Excel", "Microsoft PowerPoint", "ターミナル", "Terminal",
        "iTerm2", "Warp", "CapCut", "hindsight",
    )
    conn = sqlite3.connect(main_db)
    cur = conn.cursor()
    today = date.today()
    tstr = today.isoformat()
    marks = ",".join("?" * len(WORK_APPS))
    cands = cur.execute(
        f"SELECT local_date, device_id FROM activities WHERE local_date != ? "
        f"GROUP BY local_date, device_id "
        f"ORDER BY SUM(CASE WHEN process_name IN ({marks}) THEN duration_secs ELSE 0 END) DESC "
        f"LIMIT 8",
        (tstr, *WORK_APPS),
    ).fetchall()
    if not cands:
        conn.close()
        return
    # 模板日要像真实的开发工作日:第一应用必须是代码编辑器,且不独占全天
    # (避免选到 Word/PPT 马拉松日,合成出"单应用 48%"的一眼假)
    src_date, src_dev = cands[0]
    for d, dev in cands:
        comp = cur.execute(
            "SELECT process_name, SUM(duration_secs) FROM activities "
            "WHERE local_date = ? AND device_id = ? AND local_hour BETWEEN 7 AND 21 "
            "GROUP BY 1 ORDER BY 2 DESC",
            (d, dev),
        ).fetchall()
        if not comp:
            continue
        total = sum(sec for _, sec in comp)
        top_proc, top_sec = comp[0]
        if bucket_of(top_proc) == "code" and total > 0 and top_sec / total <= 0.45:
            src_date, src_dev = d, dev
            break

    cur.execute("DELETE FROM activities WHERE local_date = ?", (tstr,))
    rows = cur.execute(
        "SELECT started_at, ended_at, duration_secs, process_name, window_title, "
        "category_id, origin, updated_at FROM activities "
        "WHERE local_date = ? AND device_id = ? AND local_hour BETWEEN 6 AND 23 "
        "ORDER BY started_at",
        (src_date, src_dev),
    ).fetchall()

    tz = datetime.now().astimezone().tzinfo
    clock = datetime.combine(today, time(9, 0), tz)
    lunch_start = datetime.combine(today, time(12, 30), tz)
    lunch_end = datetime.combine(today, time(13, 30), tz)
    hard_end = datetime.combine(today, time(19, 30), tz)
    target = 8 * 3600  # 干满 8 小时收工,下班时刻随空档量浮动(上限 19:30)
    max_gap = timedelta(minutes=3)

    inserted, prev_src_end, lunched, worked = 0, None, False, 0
    for sa, ea, dur, proc, title, cat, origin, upd in rows:
        try:
            ss, se = parse_ts(sa), parse_ts(ea)
        except ValueError:
            continue
        if prev_src_end is not None:
            gap = ss - prev_src_end
            if gap > timedelta(0):
                clock += min(gap, max_gap)
        prev_src_end = se
        if not lunched and clock >= lunch_start:
            clock, lunched = lunch_end, True
        if worked >= target or clock >= hard_end:
            break
        ns = clock
        ne = ns + timedelta(seconds=dur)
        cutoff = hard_end if lunched else lunch_start
        ne = min(ne, cutoff)
        secs = int((ne - ns).total_seconds())
        if secs <= 0:
            continue
        cur.execute(
            "INSERT INTO activities(started_at, ended_at, duration_secs, local_date, "
            "local_hour, process_name, window_title, category_id, screenshot_path, "
            "image_hash, device_id, remote_id, updated_at, origin) "
            "VALUES (?, ?, ?, ?, ?, ?, ?, ?, NULL, NULL, ?, NULL, ?, ?)",
            (ns.isoformat(), ne.isoformat(), secs, tstr, ns.hour, proc, title,
             cat, src_dev, upd, origin),
        )
        inserted += 1
        worked += secs
        clock = ne

    total = cur.execute(
        "SELECT COALESCE(SUM(duration_secs),0) FROM activities WHERE local_date = ?",
        (tstr,),
    ).fetchone()[0]

    # 日报:段总结沿用"最近一个有日报的日子"当模板;
    # 随后把今天没有活动覆盖的时段(如深夜/晚上)内容置空,避免文不对题
    cur.execute("DELETE FROM ai_summaries WHERE local_date = ?", (tstr,))
    src2 = cur.execute(
        "SELECT local_date FROM ai_summaries WHERE local_date != ? "
        "ORDER BY local_date DESC LIMIT 1",
        (tstr,),
    ).fetchone()
    if src2:
        cols = [r[1] for r in cur.execute("PRAGMA table_info(ai_summaries)")]
        others = [c for c in cols if c != "local_date"]
        cur.execute(
            f"INSERT INTO ai_summaries(local_date, {', '.join(others)}) "
            f"SELECT ?, {', '.join(others)} FROM ai_summaries WHERE local_date = ?",
            (tstr, src2[0]),
        )
        # 有活动的时段从语料池按时段补文(模板日可能只生成过部分时段),
        # 没活动的置空——今天的日报与合成的工作日节奏严格对齐
        for rid, idx, sh, eh in cur.execute(
            "SELECT rowid, segment_idx, start_hour, end_hour FROM ai_summaries "
            "WHERE local_date = ?",
            (tstr,),
        ).fetchall():
            hours = list(range(sh, eh)) if sh < eh else list(range(sh, 24)) + list(range(0, eh))
            if not hours:
                continue
            hm = ",".join("?" * len(hours))
            active = cur.execute(
                f"SELECT COALESCE(SUM(duration_secs),0) FROM activities "
                f"WHERE local_date = ? AND local_hour IN ({hm})",
                (tstr, *hours),
            ).fetchone()[0]
            # 不足 45 分钟的时段按原生"无活动"跳过态落库(UI 显示"什么都没记录"),
            # 而不是留一张"已生成"的空壳卡片
            if active >= 45 * 60:
                cur.execute(
                    "UPDATE ai_summaries SET content = ?, status = 'ok', error = NULL "
                    "WHERE rowid = ?",
                    (pick(SEGMENT_SUMMARIES.get(idx, SEGMENT_SUMMARIES[3]), f"today{idx}"), rid),
                )
            else:
                cur.execute(
                    "UPDATE ai_summaries SET content = '', status = 'skipped_no_activity', "
                    "error = NULL WHERE rowid = ?",
                    (rid,),
                )
    conn.commit()
    conn.close()
    print(
        f"[demo] 今日合成:8 小时工作日模板 {src_date},{inserted} 条活动,"
        f"合计 {total / 3600:.1f}h。"
    )


def densify_today_memory(mem_db):
    """记忆库同步:把模板日 9:00-19:30 的文本会话按原时刻搬到今天
    (跳过午休 12:30-13:30),搜索/时间线证据在"今天"也有货。"""
    from datetime import date, datetime

    conn = sqlite3.connect(mem_db)
    cur = conn.cursor()
    today = date.today()
    tstr = today.isoformat()
    src = cur.execute(
        "SELECT local_date FROM text_sessions WHERE local_date != ? "
        "GROUP BY local_date ORDER BY COUNT(*) DESC LIMIT 1",
        (tstr,),
    ).fetchone()
    if not src:
        conn.close()
        return
    src_date = src[0]

    old_ids = [r[0] for r in cur.execute(
        "SELECT id FROM text_sessions WHERE local_date = ?", (tstr,)
    )]
    cur.executemany("DELETE FROM session_lines WHERE session_id = ?", [(i,) for i in old_ids])
    cur.execute("DELETE FROM text_sessions WHERE local_date = ?", (tstr,))

    rows = cur.execute(
        "SELECT id, started_ts, ended_ts, app_id, title, text, origin_device "
        "FROM text_sessions WHERE local_date = ? ORDER BY started_ts",
        (src_date,),
    ).fetchall()
    copied = 0
    for sid, sts, ets, app_id, title, text, odev in rows:
        try:
            ss, se = parse_ts(sts), parse_ts(ets)
        except ValueError:
            continue
        hm = ss.hour * 60 + ss.minute
        if not (9 * 60 <= hm < 19 * 60 + 30) or (12 * 60 + 30 <= hm < 13 * 60 + 30):
            continue
        ns = ss.replace(year=today.year, month=today.month, day=today.day)
        ne = se.replace(year=today.year, month=today.month, day=today.day)
        if ne < ns:
            ne = ns
        cur.execute(
            "INSERT INTO text_sessions(local_date, started_ts, ended_ts, app_id, title, "
            "text, guid, origin_device) VALUES (?, ?, ?, ?, ?, ?, NULL, ?)",
            (tstr, ns.isoformat(), ne.isoformat(), app_id, title, text, odev),
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
    print(f"[demo] 记忆同步:今天 9:00-19:30 复制 {copied} 条文本会话(模板 {src_date})。")


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
            st = parse_ts(sa)
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
        # 时段话术按实际分布挑:晚间(19-23)/深夜(23-次日 6)/白天,谁多说谁
        buckets = {"白天": 0, "晚上": 0, "深夜": 0}
        for sa, dur in bili:
            try:
                hr = parse_ts(sa).hour
            except ValueError:
                continue
            key = "深夜" if (hr >= 23 or hr < 6) else ("晚上" if hr >= 19 else "白天")
            buckets[key] += dur
        top = sorted(buckets, key=buckets.get, reverse=True)
        when = (
            f"多在{top[0]}看,{top[1]}也占一部分"
            if buckets[top[1]] > buckets[top[0]] * 0.5
            else f"基本集中在{top[0]}"
        )
        convs.append((
            "B 站观看统计",
            "这个月我在 B 站看了多久?大概看了多少次?",
            f"这个月你在 B 站累计观看约 {bili_hours} 小时,分 {times} 次看完,"
            f"{when};看得最多的是技术和数码类视频,偶尔穿插生活记录。",
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
