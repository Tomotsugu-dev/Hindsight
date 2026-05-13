// Mock 数据 —— "技术开发者一天" 风格。
// 给 demo 用，不参与主 Tauri build。
//
// 设计要点：
// - 30 天循环数据（覆盖 month 视图），按"工作日 vs 周末"两种 pattern 生成
// - 8 个内置分类，跟主应用 builtin 分类对齐（id 名一致）
// - apps 名用真实进程名（Code.exe / chrome.exe 等），让 RankedList 显示自然
// - 1 套 AI 段总结预生成内容（5 段，每段 ~150 字真实文本）

import type {
  Category,
  AppUsage,
  HourSlot,
  HourSegment,
  Settings,
  DeviceRow,
  AuthState,
  SyncStatus,
  EngineStatus,
  ModelEntry,
  RecommendedModel,
  SegmentSummaryRow,
  AppGroup,
  UnclassifiedApp,
} from "@app/api/hindsight";

// ────────────────────────────────────────────
// 分类（8 个内置 + 跟主应用一致的 id）
// ────────────────────────────────────────────

export const mockCategories: Category[] = [
  {
    id: "code",
    name: "编程",
    color: "#a78bfa",
    icon: "Code",
    builtin: true,
    apps: ["Code.exe", "cursor.exe", "WindowsTerminal.exe", "RustRover.exe"],
  },
  {
    id: "browse",
    name: "浏览",
    color: "#60a5fa",
    icon: "Globe",
    builtin: true,
    apps: ["chrome.exe", "firefox.exe", "msedge.exe"],
  },
  {
    id: "talk",
    name: "社交",
    color: "#34d399",
    icon: "MessageCircle",
    builtin: true,
    apps: ["Telegram.exe", "Teams.exe", "WeChat.exe"],
  },
  {
    id: "study",
    name: "学习",
    color: "#f59e0b",
    icon: "BookOpen",
    builtin: true,
    apps: ["Obsidian.exe", "Notion.exe"],
  },
  {
    id: "fun",
    name: "娱乐",
    color: "#ec4899",
    icon: "Gamepad2",
    builtin: true,
    apps: ["Spotify.exe", "Steam.exe", "Discord.exe"],
  },
  {
    id: "other",
    name: "其他",
    color: "#94a3b8",
    icon: "MoreHorizontal",
    builtin: true,
    apps: ["explorer.exe", "SystemSettings.exe"],
  },
];

// ────────────────────────────────────────────
// 应用清单（process_name → category）
// ────────────────────────────────────────────

interface AppDef {
  process: string;
  category: string;
}

const APPS: AppDef[] = [
  { process: "Code.exe", category: "code" },
  { process: "cursor.exe", category: "code" },
  { process: "WindowsTerminal.exe", category: "code" },
  { process: "chrome.exe", category: "browse" },
  { process: "firefox.exe", category: "browse" },
  { process: "Telegram.exe", category: "talk" },
  { process: "Teams.exe", category: "talk" },
  { process: "Obsidian.exe", category: "study" },
  { process: "Notion.exe", category: "study" },
  { process: "Spotify.exe", category: "fun" },
  { process: "Discord.exe", category: "fun" },
  { process: "Steam.exe", category: "fun" },
];

// ────────────────────────────────────────────
// 生成一天的数据
// ────────────────────────────────────────────

interface DayPlan {
  /** 时段 → 主要 app 时长分配（分钟）。每小时总和应 < 60 */
  hourlyActivity: Record<number, Array<{ process: string; minutes: number }>>;
}

// ────────────────────────────────────────────
// 7 种"日子原型"——每种结构性不同，不只是数字微调
// ────────────────────────────────────────────

/** A. 重度编程日 ~11h —— VS Code/Cursor 4-6h，全天分布 */
function archetypeHeavyCode(): DayPlan {
  return {
    hourlyActivity: {
      7: [{ process: "chrome.exe", minutes: 12 }],
      8: [
        { process: "chrome.exe", minutes: 18 },
        { process: "Telegram.exe", minutes: 6 },
      ],
      9: [
        { process: "Code.exe", minutes: 45 },
        { process: "Telegram.exe", minutes: 5 },
      ],
      10: [
        { process: "Code.exe", minutes: 48 },
        { process: "WindowsTerminal.exe", minutes: 8 },
      ],
      11: [
        { process: "Code.exe", minutes: 38 },
        { process: "chrome.exe", minutes: 15 },
        { process: "Telegram.exe", minutes: 5 },
      ],
      12: [
        { process: "chrome.exe", minutes: 20 },
        { process: "Spotify.exe", minutes: 22 },
      ],
      13: [
        { process: "cursor.exe", minutes: 42 },
        { process: "WindowsTerminal.exe", minutes: 12 },
      ],
      14: [
        { process: "cursor.exe", minutes: 50 },
        { process: "chrome.exe", minutes: 6 },
      ],
      15: [
        { process: "cursor.exe", minutes: 45 },
        { process: "WindowsTerminal.exe", minutes: 10 },
      ],
      16: [
        { process: "Code.exe", minutes: 38 },
        { process: "Telegram.exe", minutes: 10 },
      ],
      17: [
        { process: "Code.exe", minutes: 32 },
        { process: "Notion.exe", minutes: 18 },
      ],
      18: [
        { process: "Obsidian.exe", minutes: 25 },
        { process: "chrome.exe", minutes: 18 },
      ],
      19: [{ process: "Spotify.exe", minutes: 30 }],
      20: [
        { process: "chrome.exe", minutes: 35 },
        { process: "Discord.exe", minutes: 18 },
      ],
      21: [
        { process: "Code.exe", minutes: 38 },
        { process: "Discord.exe", minutes: 14 },
      ],
      22: [
        { process: "Code.exe", minutes: 32 },
        { process: "Discord.exe", minutes: 12 },
      ],
      23: [
        { process: "Steam.exe", minutes: 35 },
        { process: "Discord.exe", minutes: 18 },
      ],
    },
  };
}

/** B. 会议日 ~7h —— Teams 3h，编程零散 */
function archetypeMeetings(): DayPlan {
  return {
    hourlyActivity: {
      8: [
        { process: "chrome.exe", minutes: 12 },
        { process: "Telegram.exe", minutes: 8 },
      ],
      9: [
        { process: "Teams.exe", minutes: 50 },
        { process: "Notion.exe", minutes: 6 },
      ],
      10: [
        { process: "Teams.exe", minutes: 45 },
        { process: "chrome.exe", minutes: 10 },
      ],
      11: [
        { process: "Telegram.exe", minutes: 18 },
        { process: "Code.exe", minutes: 22 },
        { process: "chrome.exe", minutes: 10 },
      ],
      12: [
        { process: "Spotify.exe", minutes: 28 },
        { process: "chrome.exe", minutes: 14 },
      ],
      13: [
        { process: "Teams.exe", minutes: 40 },
        { process: "Notion.exe", minutes: 15 },
      ],
      14: [
        { process: "Teams.exe", minutes: 35 },
        { process: "Telegram.exe", minutes: 18 },
      ],
      15: [
        { process: "Code.exe", minutes: 25 },
        { process: "Telegram.exe", minutes: 20 },
        { process: "Notion.exe", minutes: 10 },
      ],
      16: [
        { process: "Notion.exe", minutes: 32 },
        { process: "chrome.exe", minutes: 18 },
      ],
      17: [
        { process: "Telegram.exe", minutes: 25 },
        { process: "Obsidian.exe", minutes: 22 },
      ],
      19: [{ process: "Spotify.exe", minutes: 32 }],
      20: [
        { process: "Discord.exe", minutes: 28 },
        { process: "chrome.exe", minutes: 22 },
      ],
      21: [
        { process: "Steam.exe", minutes: 38 },
        { process: "Discord.exe", minutes: 16 },
      ],
      22: [{ process: "Steam.exe", minutes: 28 }],
    },
  };
}

/** C. 写作 / 学习日 ~8h —— Notion + Obsidian 主导 */
function archetypeDesign(): DayPlan {
  return {
    hourlyActivity: {
      8: [{ process: "chrome.exe", minutes: 15 }],
      9: [
        { process: "Notion.exe", minutes: 42 },
        { process: "chrome.exe", minutes: 12 },
      ],
      10: [
        { process: "Obsidian.exe", minutes: 50 },
        { process: "Telegram.exe", minutes: 6 },
      ],
      11: [
        { process: "Notion.exe", minutes: 45 },
        { process: "chrome.exe", minutes: 12 },
      ],
      12: [
        { process: "Spotify.exe", minutes: 22 },
        { process: "chrome.exe", minutes: 18 },
      ],
      13: [
        { process: "Notion.exe", minutes: 38 },
        { process: "Telegram.exe", minutes: 12 },
      ],
      14: [
        { process: "Obsidian.exe", minutes: 48 },
        { process: "chrome.exe", minutes: 8 },
      ],
      15: [
        { process: "Obsidian.exe", minutes: 35 },
        { process: "Code.exe", minutes: 18 },
      ],
      16: [
        { process: "Code.exe", minutes: 32 },
        { process: "WindowsTerminal.exe", minutes: 10 },
      ],
      17: [
        { process: "Telegram.exe", minutes: 22 },
        { process: "Notion.exe", minutes: 20 },
      ],
      19: [
        { process: "Spotify.exe", minutes: 28 },
        { process: "chrome.exe", minutes: 18 },
      ],
      20: [
        { process: "chrome.exe", minutes: 35 },
        { process: "Discord.exe", minutes: 16 },
      ],
      21: [{ process: "Discord.exe", minutes: 32 }],
      22: [
        { process: "chrome.exe", minutes: 25 },
        { process: "Spotify.exe", minutes: 18 },
      ],
    },
  };
}

/** D. 摸鱼日 ~4h —— 总时长低，浏览 + 娱乐为主 */
function archetypeLight(): DayPlan {
  return {
    hourlyActivity: {
      9: [
        { process: "chrome.exe", minutes: 25 },
        { process: "Telegram.exe", minutes: 8 },
      ],
      10: [
        { process: "Code.exe", minutes: 18 },
        { process: "chrome.exe", minutes: 22 },
      ],
      11: [{ process: "chrome.exe", minutes: 32 }],
      14: [
        { process: "Code.exe", minutes: 25 },
        { process: "chrome.exe", minutes: 18 },
      ],
      15: [
        { process: "chrome.exe", minutes: 28 },
        { process: "Spotify.exe", minutes: 22 },
      ],
      16: [
        { process: "Telegram.exe", minutes: 18 },
        { process: "Discord.exe", minutes: 22 },
      ],
      20: [
        { process: "Steam.exe", minutes: 42 },
        { process: "Discord.exe", minutes: 12 },
      ],
      21: [
        { process: "Steam.exe", minutes: 38 },
        { process: "Discord.exe", minutes: 16 },
      ],
    },
  };
}

/** E. 深夜编程日 ~10h —— 上午轻，晚上 22-2 点编程峰值 */
function archetypeLateNight(): DayPlan {
  return {
    hourlyActivity: {
      10: [
        { process: "chrome.exe", minutes: 22 },
        { process: "Telegram.exe", minutes: 10 },
      ],
      11: [
        { process: "Code.exe", minutes: 28 },
        { process: "chrome.exe", minutes: 12 },
      ],
      13: [{ process: "Spotify.exe", minutes: 25 }],
      14: [
        { process: "cursor.exe", minutes: 32 },
        { process: "chrome.exe", minutes: 12 },
      ],
      15: [
        { process: "cursor.exe", minutes: 38 },
        { process: "WindowsTerminal.exe", minutes: 8 },
      ],
      16: [
        { process: "Code.exe", minutes: 28 },
        { process: "Telegram.exe", minutes: 12 },
      ],
      17: [
        { process: "Notion.exe", minutes: 20 },
        { process: "chrome.exe", minutes: 15 },
      ],
      19: [
        { process: "Spotify.exe", minutes: 22 },
        { process: "chrome.exe", minutes: 22 },
      ],
      20: [
        { process: "Discord.exe", minutes: 28 },
        { process: "chrome.exe", minutes: 18 },
      ],
      // 夜间编程峰值
      21: [
        { process: "Code.exe", minutes: 45 },
        { process: "Discord.exe", minutes: 10 },
      ],
      22: [
        { process: "cursor.exe", minutes: 50 },
        { process: "WindowsTerminal.exe", minutes: 8 },
      ],
      23: [
        { process: "cursor.exe", minutes: 48 },
        { process: "Discord.exe", minutes: 10 },
      ],
      0: [
        { process: "Code.exe", minutes: 42 },
        { process: "Spotify.exe", minutes: 12 },
      ],
      1: [{ process: "Code.exe", minutes: 28 }],
    },
  };
}

/** F. 周末游戏日 ~10h —— Steam 5h+ */
function archetypeWeekendGaming(): DayPlan {
  return {
    hourlyActivity: {
      10: [
        { process: "chrome.exe", minutes: 28 },
        { process: "Spotify.exe", minutes: 18 },
      ],
      11: [
        { process: "chrome.exe", minutes: 32 },
        { process: "Discord.exe", minutes: 12 },
      ],
      13: [
        { process: "Steam.exe", minutes: 48 },
        { process: "Discord.exe", minutes: 8 },
      ],
      14: [
        { process: "Steam.exe", minutes: 52 },
        { process: "Discord.exe", minutes: 6 },
      ],
      15: [
        { process: "Steam.exe", minutes: 50 },
        { process: "Discord.exe", minutes: 8 },
      ],
      16: [
        { process: "Steam.exe", minutes: 45 },
        { process: "chrome.exe", minutes: 12 },
      ],
      17: [
        { process: "Discord.exe", minutes: 38 },
        { process: "chrome.exe", minutes: 18 },
      ],
      19: [
        { process: "Spotify.exe", minutes: 30 },
        { process: "chrome.exe", minutes: 22 },
      ],
      20: [
        { process: "Steam.exe", minutes: 48 },
        { process: "Discord.exe", minutes: 10 },
      ],
      21: [
        { process: "Steam.exe", minutes: 52 },
        { process: "Discord.exe", minutes: 6 },
      ],
      22: [
        { process: "Discord.exe", minutes: 32 },
        { process: "chrome.exe", minutes: 20 },
      ],
      23: [{ process: "chrome.exe", minutes: 28 }],
    },
  };
}

/** G. 周末学习日 ~6h —— Obsidian / Notion / 阅读为主 */
function archetypeWeekendStudy(): DayPlan {
  return {
    hourlyActivity: {
      9: [
        { process: "chrome.exe", minutes: 28 },
        { process: "Spotify.exe", minutes: 18 },
      ],
      10: [
        { process: "Obsidian.exe", minutes: 42 },
        { process: "chrome.exe", minutes: 12 },
      ],
      11: [
        { process: "Obsidian.exe", minutes: 45 },
        { process: "chrome.exe", minutes: 10 },
      ],
      14: [
        { process: "chrome.exe", minutes: 38 },
        { process: "Notion.exe", minutes: 18 },
      ],
      15: [
        { process: "Notion.exe", minutes: 35 },
        { process: "Obsidian.exe", minutes: 20 },
      ],
      16: [
        { process: "Code.exe", minutes: 32 },
        { process: "chrome.exe", minutes: 18 },
      ],
      17: [
        { process: "Code.exe", minutes: 28 },
        { process: "WindowsTerminal.exe", minutes: 12 },
      ],
      20: [
        { process: "Spotify.exe", minutes: 32 },
        { process: "chrome.exe", minutes: 20 },
      ],
      21: [
        { process: "chrome.exe", minutes: 28 },
        { process: "Discord.exe", minutes: 16 },
      ],
    },
  };
}

/** 每周强度 multiplier —— 让本周 / 上周 / 上上周看着明显不同。
 *  下标按 |weekIdx| % length 取。差距加大（0.5–1.5），月视图柱状高低反差明显：
 *  - 本周 (offset 0~-6)         → 1.0  普通强度
 *  - 上周 (offset -7~-13)       → 1.5  爆肝周
 *  - 上上周 (offset -14~-20)    → 0.5  摸鱼周（柱子直接矮一半）
 *  - 3 周前 (-21~-27)           → 1.3
 *  - 4 周前 (-28~-34)           → 0.7
 *  - 5 周前 → 1.0 循环 */
const WEEK_INTENSITIES = [1.0, 1.5, 0.5, 1.3, 0.7];

/** 7 个原型按 offset 循环；保证连续两天 100% 不同。
 *
 *  关键：**每周 archetype 顺序还会旋转**，让本周 / 上周 / 上上周即使同一个"星期几"
 *  位置也是不同的 archetype——这样 WeekPage 7 天柱状图本周 vs 上周形态完全不同。
 *
 *  Week 0 (本周, offset 0~-6)：[heavy, meet, design, lateNight, light, gaming, study]
 *  Week 1 (上周, offset -7~-13)：旋转 +3 → [lateNight, light, gaming, study, heavy, meet, design]
 *  Week 2 (上上周)：旋转 +6 → [study, heavy, meet, design, lateNight, light, gaming]
 *  ... 每周 +3 旋转 (7 的互素)，cycle 7 周才回到原顺序
 */
const ARCHETYPE_CYCLE = [
  archetypeHeavyCode,    // 0
  archetypeMeetings,     // 1
  archetypeDesign,       // 2
  archetypeLateNight,    // 3
  archetypeLight,        // 4
  archetypeWeekendGaming,// 5
  archetypeWeekendStudy, // 6
];

function pickArchetype(offset: number): () => DayPlan {
  const weekIdx = Math.floor(-offset / 7); // 0=本周 1=上周 ...
  const dayInWeek = ((-offset) % 7 + 7) % 7;
  // 每往前一周旋转 +3，cycle 7 周才循环
  const rotated = (dayInWeek + weekIdx * 3) % 7;
  return ARCHETYPE_CYCLE[rotated];
}

/** 按 offset / 7 算"周 index"，决定本周强度 multiplier */
function weekIntensityFor(offset: number): number {
  const weekIdx = Math.floor(-offset / 7); // offset=-7 → weekIdx=1（上周）
  return WEEK_INTENSITIES[Math.abs(weekIdx) % WEEK_INTENSITIES.length];
}

/** 把 plan 里所有 minute 按 factor 缩放（保留 process 和小时分布） */
function scalePlan(plan: DayPlan, factor: number): DayPlan {
  if (factor === 1) return plan;
  const scaled: DayPlan = { hourlyActivity: {} };
  for (const [hourStr, acts] of Object.entries(plan.hourlyActivity)) {
    scaled.hourlyActivity[parseInt(hourStr, 10)] = acts.map((a) => ({
      process: a.process,
      minutes: Math.max(1, Math.round(a.minutes * factor)),
    }));
  }
  return scaled;
}

/** 从 DayPlan 生成 HourSlot[] + AppUsage[] */
function buildDay(plan: DayPlan): { hours: HourSlot[]; apps: AppUsage[] } {
  // 1. 聚合每小时按 category 分组
  const hours: HourSlot[] = [];
  for (let h = 0; h < 24; h++) {
    const acts = plan.hourlyActivity[h] ?? [];
    const segMap = new Map<string, number>();
    for (const a of acts) {
      const def = APPS.find((x) => x.process === a.process);
      const cat = def?.category ?? "other";
      segMap.set(cat, (segMap.get(cat) ?? 0) + a.minutes);
    }
    const segments: HourSegment[] = Array.from(segMap.entries()).map(
      ([categoryId, minutes]) => ({ categoryId, minutes }),
    );
    hours.push({ hour: h, segments });
  }

  // 2. 聚合全日 apps
  const appMap = new Map<string, number>();
  for (const acts of Object.values(plan.hourlyActivity)) {
    for (const a of acts) {
      appMap.set(a.process, (appMap.get(a.process) ?? 0) + a.minutes);
    }
  }
  const apps: AppUsage[] = Array.from(appMap.entries())
    .map(([process, minutes]) => {
      const def = APPS.find((x) => x.process === process);
      return {
        process,
        categoryId: def?.category ?? "other",
        minutes,
        iconProcess: process,
      };
    })
    .sort((a, b) => b.minutes - a.minutes);

  return { hours, apps };
}

// ────────────────────────────────────────────
// 30 天数据（offset 0 = today，-1 = 昨天 ... -29 = 29 天前）
// ────────────────────────────────────────────

export interface DayData {
  hours: HourSlot[];
  apps: AppUsage[];
}

/** 按 offset 取一天数据。
 *  - offset 0 = 今天，固定 archetypeHeavyCode（首屏视觉冲击最强）
 *  - 其它天 = 按 day-of-week 选 7 种 archetype 之一
 *  - 每周乘以一个强度系数（让"本周" vs "上周" vs "上上周"明显差异）
 *
 *  使下面 4 组数据看着差别大：
 *  - 今天 vs 昨天：不同 archetype（VS Code vs Teams 主导）
 *  - 本周 vs 上周：同样 7 个 DoW 但不同强度系数（1.0 vs 1.25 等）
 *  - 工作日 vs 周末：完全不同 archetype（编程 vs 游戏 / 学习） */
/** 应用 → 设备使用占比（self = Win 工作站，mac = MacBook）
 *  - 编程 / 工作类 (Code/Cursor/Terminal): 几乎全在 self（Windows Terminal Mac 上根本没有）
 *  - 个人 / 学习 / 娱乐: 偏 mac
 *  - 浏览 / 通讯: 大致五五开
 *
 *  调用方传 deviceId 时按对应数字缩放；不传 deviceId 时返回原值（= self + mac）。
 */
const DEVICE_SPLIT: Record<string, { self: number; mac: number }> = {
  // 编程类（Windows 工作站独占）
  "Code.exe": { self: 1.0, mac: 0.0 },
  "cursor.exe": { self: 1.0, mac: 0.0 },
  "WindowsTerminal.exe": { self: 1.0, mac: 0.0 }, // Mac 上根本没这个
  "RustRover.exe": { self: 1.0, mac: 0.0 },
  // 工作沟通（主要在 Windows 工作时间，Mac 偶尔回个消息）
  "Teams.exe": { self: 0.8, mac: 0.2 },
  // 浏览（两边都用）
  "chrome.exe": { self: 0.55, mac: 0.45 },
  "firefox.exe": { self: 0.5, mac: 0.5 },
  "msedge.exe": { self: 0.7, mac: 0.3 },
  // 通讯（个人聊天偏 Mac）
  "Telegram.exe": { self: 0.45, mac: 0.55 },
  "WeChat.exe": { self: 0.4, mac: 0.6 },
  // 笔记 / 学习（看书 / 灵感 → Mac 居多）
  "Obsidian.exe": { self: 0.3, mac: 0.7 },
  "Notion.exe": { self: 0.4, mac: 0.6 },
  // 娱乐 / 听歌（晚上 Mac 多）
  "Spotify.exe": { self: 0.25, mac: 0.75 },
  "Discord.exe": { self: 0.5, mac: 0.5 },
  "Steam.exe": { self: 0.9, mac: 0.1 }, // 游戏几乎全在 Windows
};

/** 按 deviceId 算单个 app 的缩放系数；undefined / "" / "all" 返 1.0（不过滤）。 */
function deviceFactor(process: string, deviceId?: string): number {
  if (!deviceId || deviceId === "all") return 1.0;
  const split = DEVICE_SPLIT[process] ?? { self: 0.5, mac: 0.5 };
  if (deviceId === "demo-self") return split.self;
  if (deviceId === "demo-mac") return split.mac;
  return 1.0; // 未知 deviceId fallback 不过滤
}

/** 把整个 plan 按设备占比缩放；缩到 0 的 app 直接过滤掉。 */
function filterPlanByDevice(plan: DayPlan, deviceId?: string): DayPlan {
  if (!deviceId || deviceId === "all") return plan;
  const filtered: DayPlan = { hourlyActivity: {} };
  for (const [hourStr, acts] of Object.entries(plan.hourlyActivity)) {
    const scaled = acts
      .map((a) => ({
        process: a.process,
        minutes: Math.round(a.minutes * deviceFactor(a.process, deviceId)),
      }))
      .filter((a) => a.minutes > 0);
    if (scaled.length > 0) {
      filtered.hourlyActivity[parseInt(hourStr, 10)] = scaled;
    }
  }
  return filtered;
}

export function mockDayFor(offset: number, deviceId?: string): DayData {
  const archetype = pickArchetype(offset);
  const intensity = weekIntensityFor(offset);
  let plan = scalePlan(archetype(), intensity);
  plan = filterPlanByDevice(plan, deviceId);
  return buildDay(plan);
}

// ────────────────────────────────────────────
// AI 段总结（预生成 5 段，给 DailyTab 渲染）
// ────────────────────────────────────────────

const SEG_MODEL = "qwen2.5-vl-3b-instruct-q4_k_m.gguf";

const SEG_CONTENT_ZH = [
  "深夜时段基本无活动。00:30 短暂打开 Chrome 看了几篇技术文章后合上电脑。",
  "上午集中精力在 VS Code 中完成 React 组件的渲染性能问题排查——主要在 Today 页的 HourlyChart 上做 useMemo 优化。中间被 Telegram 打断 3 次（产品同事关于本周 demo 准备的对齐讨论），但每次都很简短。Chrome 主要用来查 React 18 → 19 的 useTransition 行为变化、看 MDN 的 web-perf 文档。临近午饭前提交了 1 次代码，标题 \"perf(today): memoize hourly aggregation\"。",
  "下午切到 Cursor 启动新 feature 开发——landing page 嵌入真 React demo 的架构原型。Windows Terminal 频繁使用（npm / git / vite），主要在迭代 vite.config.demo.ts 配置。Obsidian 里整理了一份 hero 区设计稿的间距 token 备忘。期间在 Telegram 跟前端 team lead 讨论了「是 iframe 还是 shadow DOM 嵌入 demo」的取舍，最终倾向 iframe 方案。",
  "傍晚转到 Obsidian 整理本周思考笔记——主要是 demo 架构决策的存档。短暂打开 Notion 看了一会儿团队周报。Spotify 一直在放歌（lo-fi study mix），晚饭后看了 30 分钟 YouTube 关于 Tauri 2 自动更新机制的 talk。",
  "夜间回到 VS Code 推了一版 demo 的 fixtures，然后切 Steam 玩了一会儿独立游戏放松。Discord 跟朋友闲聊。23:30 左右把 Steam 关掉准备睡觉。",
];

const SEG_CONTENT_EN = [
  "Almost no activity during the late-night window. Briefly opened Chrome around 00:30 to skim a couple of tech articles, then closed the laptop.",
  "Focused morning in VS Code, drilling into a React rendering perf issue on the Today page's HourlyChart — mostly tightening useMemo dependencies. Got pulled into Telegram three times for short syncs with the product team about this week's demo prep. Chrome was for reading up on React 18 → 19 useTransition behavior changes and the MDN web-perf docs. Pushed one commit just before lunch: \"perf(today): memoize hourly aggregation\".",
  "Switched to Cursor in the afternoon to start a new feature — prototyping the architecture for embedding a real React demo in the landing page. Windows Terminal was busy with npm / git / vite, mostly iterating on vite.config.demo.ts. Used Obsidian to jot down spacing tokens for the hero mockup. Had a short Telegram thread with the front-end lead weighing iframe vs shadow DOM for embedding the demo — landed on iframe.",
  "Evening shifted to Obsidian for weekly reflection notes — mainly archiving demo architecture decisions. Quick peek at Notion for the team weekly. Spotify ran a lo-fi study mix in the background. After dinner, watched ~30 minutes of a YouTube talk on Tauri 2's auto-updater design.",
  "Back in VS Code at night, pushed a new pass on the demo fixtures, then unwound with an indie game on Steam. Chatted on Discord with friends. Closed Steam around 23:30 and called it.",
];

const SEG_CONTENT_JA = [
  "深夜帯はほぼアクティビティなし。00:30 ごろ Chrome を少し開き、技術記事を数本だけ流し読みしてラップトップを閉じました。",
  "午前は VS Code に集中、Today ページの HourlyChart の React レンダリング性能調査に取り組み、主に useMemo の依存配列を整理しました。途中で Telegram に 3 回呼ばれ、プロダクトチームと今週のデモ準備について短い同期。Chrome は React 18 → 19 の useTransition の挙動変更と MDN の web-perf ドキュメント参照に使用。昼食前にコミット 1 件、メッセージは「perf(today): memoize hourly aggregation」。",
  "午後は Cursor に切り替えて新機能の開発に着手——ランディングページに本物の React デモを埋め込むアーキテクチャの試作です。Windows Terminal が npm / git / vite で頻繁に活躍し、主に vite.config.demo.ts を反復調整。Obsidian にヒーローのデザインスペーシングトークンをメモ。Telegram でフロントエンドリードと「埋め込みは iframe か shadow DOM か」のトレードオフを議論し、最終的に iframe 案に着地。",
  "夕方は Obsidian で週次の振り返りメモを整理——主にデモのアーキテクチャ決定のアーカイブです。Notion でチーム週報を軽く確認。Spotify は背景で lo-fi study mix を流しっぱなし。夕食後は Tauri 2 の自動更新機構に関する YouTube トークを 30 分ほど視聴。",
  "夜は VS Code に戻ってデモ fixtures を一回し更新。その後 Steam でインディーゲームを軽くプレイしてリラックス。Discord で友人と雑談。23:30 ごろに Steam を閉じて就寝へ。",
];

const SEG_LABELS_ZH = ["深夜", "上午", "下午", "晚上", "深夜"];
const SEG_LABELS_EN = ["Late night", "Morning", "Afternoon", "Evening", "Late night"];
const SEG_LABELS_JA = ["深夜", "午前", "午後", "夜", "深夜"];

const SEG_HOURS: Array<{ startHour: number; endHour: number }> = [
  { startHour: 0, endHour: 6 },
  { startHour: 6, endHour: 12 },
  { startHour: 12, endHour: 18 },
  { startHour: 18, endHour: 22 },
  { startHour: 22, endHour: 24 },
];

function buildSegments(
  labels: readonly string[],
  contents: readonly string[],
): SegmentSummaryRow[] {
  return labels.map((label, i) => ({
    source: "daily",
    localDate: todayStr(),
    segmentIdx: i,
    label,
    startHour: SEG_HOURS[i].startHour,
    endHour: SEG_HOURS[i].endHour,
    content: contents[i],
    model: SEG_MODEL,
    status: "ok",
    error: null,
    generatedAt: new Date().toISOString(),
  }));
}

/** 按当前 i18n locale 返回 5 段总结。fallback = en。 */
export function dailySegmentsForLocale(locale: string): SegmentSummaryRow[] {
  const lng = (locale || "").toLowerCase();
  if (lng.startsWith("zh")) return buildSegments(SEG_LABELS_ZH, SEG_CONTENT_ZH);
  if (lng.startsWith("ja")) return buildSegments(SEG_LABELS_JA, SEG_CONTENT_JA);
  return buildSegments(SEG_LABELS_EN, SEG_CONTENT_EN);
}

/** AI 设置里的 "About you / あなたについて / 关于你" 用户简介。按当前 i18n 切。 */
export function userBriefForLocale(locale: string): string {
  const lng = (locale || "").toLowerCase();
  if (lng.startsWith("ja"))
    return "フルスタックエンジニア。主に React + Rust のプロジェクトに取り組んでいます。";
  if (lng.startsWith("zh")) return "全栈开发者，主要做 React + Rust 项目。";
  return "Full-stack engineer, mostly working on React + Rust projects.";
}

/** 默认（中文）。api-mock 初始化 state 时用；运行时通过 dailySegmentsForLocale 按 i18n 切。 */
export const mockDailySegments: SegmentSummaryRow[] = buildSegments(
  SEG_LABELS_ZH,
  SEG_CONTENT_ZH,
);

function todayStr(): string {
  const d = new Date();
  return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}-${String(d.getDate()).padStart(2, "0")}`;
}

// ────────────────────────────────────────────
// 默认 settings
// ────────────────────────────────────────────

export const mockSettings: Settings = {
  captureEnabled: true,
  screenshotEnabled: true,
  captureIntervalSeconds: 30,
  screenshotPath: "C:\\Users\\demo\\AppData\\Roaming\\Hindsight\\screenshots",
  workHoursEnabled: true,
  workRanges: [
    { start: "09:00", end: "12:00" },
    { start: "13:00", end: "18:00" },
  ],
  autoStart: true,
  showWindowOnAutoStart: false,
  retentionDays: 90,
  googleClientId: "",
  googleClientSecret: "",
  privacyUrlKeywords: ["login", "signin", "password", "auth"],
  privacyAppKeywords: ["KeePass", "1Password"],
  minimizeToTray: true,
  autoUpdateEnabled: true,
  autoUpdateInterval: "daily",
  lastUpdateCheckAt: new Date().toISOString(),
  idleThresholdSeconds: 300,
  ai: {
    endpoint: "",
    model: "",
    apiKey: "",
    externalEnabled: false,
    externalProvider: "openai",
    userBrief: "全栈开发者，主要做 React + Rust 项目。",
    segments: [
      { label: "深夜", startHour: 0, endHour: 6, color: "" },
      { label: "上午", startHour: 6, endHour: 12, color: "" },
      { label: "下午", startHour: 12, endHour: 18, color: "" },
      { label: "晚上", startHour: 18, endHour: 22, color: "" },
      { label: "深夜", startHour: 22, endHour: 24, color: "" },
    ],
    excludedCategories: [],
    maxImagesPerSegment: 1024,
    dedupThreshold: 0.95,
    modelsPath: "C:\\Users\\demo\\AppData\\Roaming\\Hindsight\\ai\\models",
    activeMain: "Qwen2.5-VL-3B-Instruct-Q4_K_M.gguf",
    activeMmproj: "mmproj-Qwen2.5-VL-3B-Instruct-f16.gguf",
    describeMain: "",
    describeMmproj: "",
    summaryMain: "",
    summaryMmproj: "",
    promptLanguage: "zh",
    promptOverrides: { systemZh: "", systemEn: "", systemJa: "" },
    imageDescribeOverrides: { systemZh: "", systemEn: "", systemJa: "" },
    batchSize: null,
    parallelSlots: null,
    ctxSize: null,
    describeBatchSize: null,
    describeParallelSlots: null,
    describeCtxSize: null,
    summaryBatchSize: null,
    summaryParallelSlots: null,
    summaryCtxSize: null,
  },
};

// ────────────────────────────────────────────
// 设备 / 同步状态
// ────────────────────────────────────────────

export const mockDevices: DeviceRow[] = [
  {
    deviceId: "demo-self",
    displayName: "我的工作站",
    color: "#7c3aed",
    icon: "Monitor",
    os: "windows",
    lastSeenAt: new Date().toISOString(),
    isSelf: true,
  },
  {
    deviceId: "demo-mac",
    displayName: "MacBook Pro",
    color: "#06b6d4",
    icon: "Laptop",
    os: "macos",
    lastSeenAt: new Date(Date.now() - 1000 * 60 * 45).toISOString(),
    isSelf: false,
  },
];

export const mockAuthState: AuthState = {
  signedIn: false,
  uid: null,
  email: null,
  configured: false,
};

export const mockSyncStatus: SyncStatus = {
  running: false,
  lastPushedAt: new Date(Date.now() - 1000 * 60 * 30).toISOString(),
  lastPulledAt: new Date(Date.now() - 1000 * 60 * 30).toISOString(),
  lastError: null,
  pending: 0,
  deadLetter: 0,
};

// ────────────────────────────────────────────
// AI Engine / Models（demo 假"已 ready"状态）
// ────────────────────────────────────────────

export const mockEngineStatus: EngineStatus = {
  installed: true,
  installedVersion: "b4720",
  currentPin: "b4720",
  platformId: "win-cuda-12.4-x64",
  assetName: "llama-b4720-bin-win-cuda-cu12.4-x64.zip",
  estimatedBytes: 220 * 1024 * 1024,
  runtime: {
    state: "running",
    port: 8088,
    error: null,
    idleSecondsRemaining: null,
  },
  embeddingRuntime: {
    installed: true,
    installedVersion: "1.20.1",
    currentPin: "1.20.1",
    estimatedBytes: 28 * 1024 * 1024,
  },
  protectionDegraded: null,
  systemVram: { totalGb: 12, source: "discrete" },
};

export const mockLocalModels: ModelEntry[] = [
  {
    filename: "Qwen2.5-VL-3B-Instruct-Q4_K_M.gguf",
    path: "C:\\Users\\demo\\AppData\\Roaming\\Hindsight\\ai\\models\\Qwen2.5-VL-3B-Instruct-Q4_K_M.gguf",
    sizeBytes: 2_100_000_000,
    isMmproj: false,
  },
  {
    filename: "mmproj-Qwen2.5-VL-3B-Instruct-f16.gguf",
    path: "C:\\Users\\demo\\AppData\\Roaming\\Hindsight\\ai\\models\\mmproj-Qwen2.5-VL-3B-Instruct-f16.gguf",
    sizeBytes: 1_300_000_000,
    isMmproj: true,
  },
];

export const mockRecommendedModels: RecommendedModel[] = [
  {
    displayName: "Qwen2.5-VL 3B (Q4_K_M)",
    repo: "ggml-org/Qwen2.5-VL-3B-Instruct-GGUF",
    mainFile: "Qwen2.5-VL-3B-Instruct-Q4_K_M.gguf",
    mainBytes: 2_100_000_000,
    mmprojFile: "mmproj-Qwen2.5-VL-3B-Instruct-f16.gguf",
    mmprojBytes: 1_300_000_000,
    logoUrl: null,
    vision: true,
    brand: "Qwen",
  },
];

// ────────────────────────────────────────────
// App groups / unclassified（Categories 页用）
// ────────────────────────────────────────────

export const mockAppGroups: AppGroup[] = [];

export const mockUnclassifiedApps: UnclassifiedApp[] = [
  {
    processName: "discord.exe",
    minutes: 86,
    lastSeenAt: new Date(Date.now() - 1000 * 60 * 60).toISOString(),
  },
  {
    processName: "GitHubDesktop.exe",
    minutes: 22,
    lastSeenAt: new Date(Date.now() - 1000 * 60 * 60 * 3).toISOString(),
  },
];
