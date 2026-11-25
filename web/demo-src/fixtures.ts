// Mock 数据 —— "技术开发者一天" 风格。
// 给 demo 用，不参与主 Tauri build。
//
// 设计要点：
// - Cyclic day data generated per offset (last 30 days cover the month view,
//   full year covers the All-time heatmap), weekday vs weekend patterns
// - 8 个内置分类，跟主应用 builtin 分类对齐（id 名一致）
// - apps 名用真实进程名（Code.exe / chrome.exe 等），让 RankedList 显示自然
// - 1 套 AI 段总结预生成内容（5 段，每段 ~150 字真实文本）

import type {
  Category,
  AppUsage,
  HourSlot,
  CategoryTime,
  Settings,
  DeviceRow,
  AuthState,
  SyncStatus,
  EngineStatus,
  ModelEntry,
  RecommendedModel,
  SegmentSummaryRow,
  AiSegment,
  AppGroup,
  SuperCategory,
  UnclassifiedApp,
} from "@app/api/hindsight";
import { DEFAULT_DOWNLOAD_SOURCES } from "@app/config/downloadSources";
import { retranslateDefaultSegments } from "@app/utils/defaultSegments";

// ────────────────────────────────────────────
// Category ids match the app defaults (code, office, workchat, browse,
// talk, video, game)
// ────────────────────────────────────────────

export const mockCategories: Category[] = [
  {
    id: "code",
    name: "编程",
    color: "#a78bfa",
    icon: "Code",
    builtin: false,
    apps: ["Code.exe", "cursor.exe", "WindowsTerminal.exe", "RustRover.exe"],
    superCategoryId: "work",
  },
  {
    id: "office",
    name: "办公",
    color: "#0ea5e9",
    icon: "Briefcase",
    builtin: false,
    apps: ["Obsidian.exe", "Notion.exe"],
    superCategoryId: "work",
  },
  {
    id: "workchat",
    name: "工作沟通",
    color: "#14b8a6",
    icon: "MessageCircle",
    builtin: false,
    apps: ["Teams.exe"],
    superCategoryId: "work",
  },
  {
    id: "browse",
    name: "浏览",
    color: "#60a5fa",
    icon: "Globe",
    builtin: false,
    apps: ["chrome.exe", "firefox.exe", "msedge.exe"],
    superCategoryId: "browse",
  },
  {
    id: "talk",
    name: "社交",
    color: "#34d399",
    icon: "MessageCircle",
    builtin: false,
    apps: ["Telegram.exe", "Discord.exe", "WeChat.exe"],
    superCategoryId: "social",
  },
  {
    id: "video",
    name: "影音",
    color: "#a855f7",
    icon: "Film",
    builtin: false,
    apps: ["Spotify.exe"],
    superCategoryId: "play",
  },
  {
    id: "game",
    name: "游戏",
    color: "#fb7185",
    icon: "Gamepad2",
    builtin: false,
    apps: ["Steam.exe"],
    superCategoryId: "play",
  },
  {
    id: "other",
    name: "其他",
    color: "#94a3b8",
    icon: "MoreHorizontal",
    builtin: false,
    apps: ["explorer.exe", "SystemSettings.exe"],
    superCategoryId: null,
  },
];

// 跟 Rust migration v29 / v31 / v32 / v33 seed 出来的默认大类一致
export const mockSuperCategories: SuperCategory[] = [
  { id: "work", name: "工作", color: "#8b5cf6", icon: "Briefcase", sortOrder: 0 },
  { id: "play", name: "娱乐", color: "#ec4899", icon: "Sparkles", sortOrder: 1 },
  { id: "social", name: "社交", color: "#34d399", icon: "MessageCircle", sortOrder: 2 },
  { id: "browse", name: "浏览", color: "#60a5fa", icon: "Globe", sortOrder: 3 },
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
  { process: "Discord.exe", category: "talk" },
  { process: "Teams.exe", category: "workchat" },
  { process: "Obsidian.exe", category: "office" },
  { process: "Notion.exe", category: "office" },
  { process: "Spotify.exe", category: "video" },
  { process: "Steam.exe", category: "game" },
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
  archetypeHeavyCode, // 0
  archetypeMeetings, // 1
  archetypeDesign, // 2
  archetypeLateNight, // 3
  archetypeLight, // 4
  archetypeWeekendGaming, // 5
  archetypeWeekendStudy, // 6
];

function pickArchetype(offset: number): () => DayPlan {
  const weekIdx = Math.floor(-offset / 7); // 0=本周 1=上周 ...
  const dayInWeek = ((-offset % 7) + 7) % 7;
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
    const segments: CategoryTime[] = Array.from(segMap.entries()).map(([categoryId, minutes]) => ({
      categoryId,
      secs: minutes * 60,
    }));
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
      const categoryId = def?.category ?? "other";
      return {
        displayName: process,
        groupId: process,
        categoryId,
        minutes,
        iconProcess: process,
        byCategory: [{ categoryId, secs: minutes * 60 }],
      };
    })
    .sort((a, b) => b.minutes - a.minutes);

  return { hours, apps };
}

// ────────────────────────────────────────────
// Day data by offset (0 = today, negative = past days; the All-time year
// view reaches back to -364)
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
  "早上活动不多。7 点多在 Chrome 里看了会儿新闻和 Hacker News，吃早饭时在 Telegram 回了几条消息。",
  '上午集中精力在 VS Code 中完成 React 组件的渲染性能问题排查——主要在 Today 页的 HourlyChart 上做 useMemo 优化。中间被 Telegram 打断 3 次（产品同事关于本周 demo 准备的对齐讨论），但每次都很简短。Chrome 主要用来查 React 18 → 19 的 useTransition 行为变化、看 MDN 的 web-perf 文档。临近午饭前提交了 1 次代码，标题 "perf(today): memoize hourly aggregation"。',
  "下午切到 Cursor 启动新 feature 开发——landing page 嵌入真 React demo 的架构原型。Windows Terminal 频繁使用（npm / git / vite），主要在迭代 vite.config.demo.ts 配置。Obsidian 里整理了一份 hero 区设计稿的间距 token 备忘。期间在 Telegram 跟前端 team lead 讨论了「是 iframe 还是 shadow DOM 嵌入 demo」的取舍，最终倾向 iframe 方案。",
  "傍晚转到 Obsidian 整理本周思考笔记——主要是 demo 架构决策的存档。短暂打开 Notion 看了一会儿团队周报。Spotify 一直在放歌（lo-fi study mix），晚饭后看了 30 分钟 YouTube 关于 Tauri 2 自动更新机制的 talk。夜间回到 VS Code 推了一版 demo 的 fixtures，然后切 Steam 玩了一会儿独立游戏放松。Discord 跟朋友闲聊。23:30 左右把 Steam 关掉准备睡觉。",
];

const SEG_CONTENT_EN = [
  "Almost no activity during the late-night window. Briefly opened Chrome around 00:30 to skim a couple of tech articles, then closed the laptop.",
  "A light start. A little after 7, skimmed the news and Hacker News in Chrome, and answered a few Telegram messages over breakfast.",
  "Focused morning in VS Code, drilling into a React rendering perf issue on the Today page's HourlyChart — mostly tightening useMemo dependencies. Got pulled into Telegram three times for short syncs with the product team about this week's demo prep. Chrome was for reading up on React 18 → 19 useTransition behavior changes and the MDN web-perf docs. Pushed one commit just before lunch: \"perf(today): memoize hourly aggregation\".",
  "Switched to Cursor in the afternoon to start a new feature — prototyping the architecture for embedding a real React demo in the landing page. Windows Terminal was busy with npm / git / vite, mostly iterating on vite.config.demo.ts. Used Obsidian to jot down spacing tokens for the hero mockup. Had a short Telegram thread with the front-end lead weighing iframe vs shadow DOM for embedding the demo — landed on iframe.",
  "Evening shifted to Obsidian for weekly reflection notes — mainly archiving demo architecture decisions. Quick peek at Notion for the team weekly. Spotify ran a lo-fi study mix in the background. After dinner, watched ~30 minutes of a YouTube talk on Tauri 2's auto-updater design. Back in VS Code at night, pushed a new pass on the demo fixtures, then unwound with an indie game on Steam. Chatted on Discord with friends. Closed Steam around 23:30 and called it.",
];

const SEG_CONTENT_JA = [
  "深夜帯はほぼアクティビティなし。00:30 ごろ Chrome を少し開き、技術記事を数本だけ流し読みしてラップトップを閉じました。",
  "朝は軽めのスタート。7 時過ぎに Chrome でニュースと Hacker News を眺め、朝食をとりながら Telegram で数件返信。",
  "午前は VS Code に集中、Today ページの HourlyChart の React レンダリング性能調査に取り組み、主に useMemo の依存配列を整理しました。途中で Telegram に 3 回呼ばれ、プロダクトチームと今週のデモ準備について短い同期。Chrome は React 18 → 19 の useTransition の挙動変更と MDN の web-perf ドキュメント参照に使用。昼食前にコミット 1 件、メッセージは「perf(today): memoize hourly aggregation」。",
  "午後は Cursor に切り替えて新機能の開発に着手——ランディングページに本物の React デモを埋め込むアーキテクチャの試作です。Windows Terminal が npm / git / vite で頻繁に活躍し、主に vite.config.demo.ts を反復調整。Obsidian にヒーローのデザインスペーシングトークンをメモ。Telegram でフロントエンドリードと「埋め込みは iframe か shadow DOM か」のトレードオフを議論し、最終的に iframe 案に着地。",
  "夕方は Obsidian で週次の振り返りメモを整理——主にデモのアーキテクチャ決定のアーカイブです。Notion でチーム週報を軽く確認。Spotify は背景で lo-fi study mix を流しっぱなし。夕食後は Tauri 2 の自動更新機構に関する YouTube トークを 30 分ほど視聴。夜は VS Code に戻ってデモ fixtures を一回し更新。その後 Steam でインディーゲームを軽くプレイしてリラックス。Discord で友人と雑談。23:30 ごろに Steam を閉じて就寝へ。",
];

const SEG_CONTENT_TW = [
  "深夜時段幾乎沒有活動。00:30 短暫開啟 Chrome 看了幾篇技術文章後就闔上電腦。",
  "早上活動不多。7 點多在 Chrome 看了一下新聞和 Hacker News，吃早餐時在 Telegram 回了幾則訊息。",
  '上午專注在 VS Code 裡排查 React 元件的渲染效能問題——主要是替 Today 頁的 HourlyChart 做 useMemo 最佳化。中間被 Telegram 打斷 3 次（產品同事討論本週 demo 準備的對齊），但每次都很簡短。Chrome 主要用來查 React 18 → 19 的 useTransition 行為變化、看 MDN 的 web-perf 文件。快到午飯前提交了 1 次程式碼，標題 "perf(today): memoize hourly aggregation"。',
  "下午切到 Cursor 開始新功能開發——讓 landing page 嵌入真正 React demo 的架構原型。Windows Terminal 用得很頻繁（npm / git / vite），主要在反覆調整 vite.config.demo.ts 設定。在 Obsidian 整理了一份 hero 區設計稿的間距 token 備忘。期間在 Telegram 與前端 team lead 討論「用 iframe 還是 shadow DOM 嵌入 demo」的取捨，最後傾向 iframe 方案。",
  "傍晚轉到 Obsidian 整理本週思考筆記——主要是 demo 架構決策的存檔。短暫開啟 Notion 看了一下團隊週報。Spotify 一直在播歌（lo-fi study mix），晚飯後看了 30 分鐘 YouTube 上關於 Tauri 2 自動更新機制的演講。夜間回到 VS Code 推了一版 demo 的 fixtures，然後切到 Steam 玩了一下獨立遊戲放鬆。Discord 跟朋友閒聊。23:30 左右關掉 Steam 準備睡覺。",
];

const SEG_CONTENT_PT = [
  "Quase nenhuma atividade na madrugada. Por volta das 00:30, abri o Chrome rapidamente para ler alguns artigos técnicos e depois fechei o notebook.",
  "Começo tranquilo. Pouco depois das 7h, passei os olhos nas notícias e no Hacker News pelo Chrome e respondi algumas mensagens no Telegram durante o café da manhã.",
  'Manhã focada no VS Code, investigando um problema de desempenho de renderização em React no HourlyChart da página Today — principalmente ajustando as dependências do useMemo. Fui interrompido três vezes no Telegram por conversas rápidas com o time de produto sobre a preparação do demo desta semana. O Chrome serviu para ler sobre as mudanças de comportamento do useTransition do React 18 → 19 e a documentação de web-perf da MDN. Fiz um commit pouco antes do almoço: "perf(today): memoize hourly aggregation".',
  "À tarde, mudei para o Cursor e comecei uma nova funcionalidade — um protótipo da arquitetura para incorporar um demo real em React na landing page. O Windows Terminal ficou ocupado com npm / git / vite, principalmente iterando sobre o vite.config.demo.ts. Usei o Obsidian para anotar os tokens de espaçamento do mockup do hero. Tive uma conversa curta no Telegram com o líder de front-end sobre iframe vs shadow DOM para incorporar o demo — acabamos escolhendo iframe.",
  "No fim da tarde, passei para o Obsidian para escrever notas de reflexão da semana — principalmente registrando as decisões de arquitetura do demo. Dei uma olhada rápida no Notion para ver o semanal da equipe. O Spotify tocou um mix de lo-fi para estudar ao fundo. Depois do jantar, assisti a cerca de 30 minutos de uma palestra no YouTube sobre o design do atualizador automático do Tauri 2. À noite voltei ao VS Code e enviei mais uma rodada de ajustes nos fixtures do demo, depois relaxei com um jogo indie no Steam. Conversei com amigos no Discord. Fechei o Steam por volta das 23:30 e fui dormir.",
];

const SEG_CONTENT_ES = [
  "Casi sin actividad durante la madrugada. Hacia las 00:30 abrí Chrome un momento para hojear un par de artículos técnicos y luego cerré el portátil.",
  "Un comienzo tranquilo. Poco después de las 7, ojeé las noticias y Hacker News en Chrome y respondí algunos mensajes de Telegram mientras desayunaba.",
  'Mañana enfocada en VS Code, investigando un problema de rendimiento de renderizado de React en el HourlyChart de la página Today — sobre todo ajustando las dependencias de useMemo. Me interrumpieron tres veces en Telegram con charlas breves con el equipo de producto sobre la preparación del demo de esta semana. Chrome sirvió para leer sobre los cambios de comportamiento de useTransition de React 18 → 19 y la documentación de web-perf de MDN. Hice un commit poco antes del almuerzo: "perf(today): memoize hourly aggregation".',
  "Por la tarde cambié a Cursor para empezar una nueva funcionalidad — un prototipo de la arquitectura para incrustar un demo real de React en la landing page. Windows Terminal estuvo muy activo con npm / git / vite, sobre todo iterando sobre vite.config.demo.ts. Usé Obsidian para apuntar los tokens de espaciado del mockup del hero. Tuve una charla breve en Telegram con el líder de front-end sobre iframe vs shadow DOM para incrustar el demo — nos quedamos con iframe.",
  "Al caer la tarde pasé a Obsidian para escribir notas de reflexión de la semana — sobre todo archivando las decisiones de arquitectura del demo. Eché un vistazo rápido a Notion para ver el semanal del equipo. Spotify puso de fondo una mezcla lo-fi para estudiar. Después de cenar vi unos 30 minutos de una charla de YouTube sobre el diseño del actualizador automático de Tauri 2. De noche volví a VS Code y subí otra ronda de cambios a los fixtures del demo; luego me relajé con un juego indie en Steam. Charlé con amigos en Discord. Cerré Steam hacia las 23:30 y me fui a dormir.",
];

/** 和真实应用在中文系统上首次启动时的默认时段一样（config.rs 的 default_segments_for）。 */
const DEFAULT_AI_SEGMENTS: AiSegment[] = [
  { label: "深夜", startHour: 0, endHour: 6, color: "" },
  { label: "早上", startHour: 6, endHour: 9, color: "" },
  { label: "上午", startHour: 9, endHour: 12, color: "" },
  { label: "下午", startHour: 12, endHour: 18, color: "" },
  { label: "晚上", startHour: 18, endHour: 24, color: "" },
];

function buildSegments(locale: string, contents: readonly string[]): SegmentSummaryRow[] {
  return retranslateDefaultSegments(DEFAULT_AI_SEGMENTS, locale).map((seg, i) => ({
    source: "daily",
    localDate: todayStr(),
    segmentIdx: i,
    label: seg.label,
    startHour: seg.startHour,
    endHour: seg.endHour,
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
  if (/^zh[-_]?(tw|hk|mo|hant)/.test(lng)) return buildSegments(lng, SEG_CONTENT_TW);
  if (lng.startsWith("zh")) return buildSegments(lng, SEG_CONTENT_ZH);
  if (lng.startsWith("ja")) return buildSegments(lng, SEG_CONTENT_JA);
  if (lng.startsWith("pt")) return buildSegments(lng, SEG_CONTENT_PT);
  if (lng.startsWith("es")) return buildSegments(lng, SEG_CONTENT_ES);
  return buildSegments(lng, SEG_CONTENT_EN);
}

const WEEKLY_CONTENT = {
  zh: "本周的重心是网页 demo。先在 VS Code 里排查 Today 页的渲染性能，之后换到 Cursor，把真正的 React demo 嵌进官网首页，最后选了 iframe 方案。\n\n沟通集中在 Telegram，大多是和产品同事对齐 demo 的准备，每次都很短。晚上主要在 Obsidian 整理笔记，休息时多是 Steam 和 YouTube。\n\n下周可以把上午的整块时间留给写代码，消息集中到午饭后再回。",
  tw: "本週的重心是網頁 demo。先在 VS Code 裡排查 Today 頁的渲染效能，之後換到 Cursor，把真正的 React demo 嵌進官網首頁，最後選了 iframe 方案。\n\n溝通集中在 Telegram，大多是和產品同事對齊 demo 的準備，每次都很短。晚上主要在 Obsidian 整理筆記，休息時多是 Steam 和 YouTube。\n\n下週可以把上午的整塊時間留給寫程式，訊息集中到午飯後再回。",
  en: "This week centered on the web demo. First came a rendering performance issue on the Today page in VS Code. Then the work moved to Cursor, to embed a real React demo in the landing page; the final choice was an iframe.\n\nMessages stayed in Telegram, mostly short syncs with the product team about the demo. Evenings went to notes in Obsidian, and breaks went to Steam and YouTube.\n\nNext week, keep the mornings for coding and answer messages after lunch.",
  ja: "今週の中心は Web デモでした。まず VS Code で Today ページのレンダリング性能を調べ、その後 Cursor に移って本物の React デモをランディングページに埋め込み、最終的に iframe 方式に決めました。\n\nやり取りは Telegram に集中し、主にプロダクトチームとのデモ準備の短い確認でした。夜は Obsidian でメモを整理し、休憩中は Steam と YouTube が中心でした。\n\n来週は午前中をまとめてコーディングに使い、メッセージは昼食後にまとめて返すとよさそうです。",
  pt: "Esta semana girou em torno do demo web. Primeiro, investiguei no VS Code um problema de desempenho de renderização na página Today. Depois passei para o Cursor para incorporar um demo real em React na landing page; a escolha final foi um iframe.\n\nAs conversas ficaram no Telegram, quase sempre alinhamentos rápidos com o time de produto sobre o demo. As noites foram para notas no Obsidian, e as pausas, para Steam e YouTube.\n\nNa próxima semana, vale reservar as manhãs para programar e responder mensagens depois do almoço.",
  es: "Esta semana giró en torno al demo web. Primero investigué en VS Code un problema de rendimiento de renderizado en la página Today. Después pasé a Cursor para incrustar un demo real de React en la landing page; al final elegimos un iframe.\n\nLas conversaciones se quedaron en Telegram, casi siempre charlas breves con el equipo de producto sobre el demo. Las noches fueron para notas en Obsidian, y los descansos, para Steam y YouTube.\n\nLa próxima semana conviene reservar las mañanas para programar y responder mensajes después de comer.",
};

/** Returns the sample weekly report text in the given UI language. Falls back to English. */
export function weeklyContentForLocale(locale: string): string {
  const lng = (locale || "").toLowerCase();
  if (/^zh[-_]?(tw|hk|mo|hant)/.test(lng)) return WEEKLY_CONTENT.tw;
  if (lng.startsWith("zh")) return WEEKLY_CONTENT.zh;
  if (lng.startsWith("ja")) return WEEKLY_CONTENT.ja;
  if (lng.startsWith("pt")) return WEEKLY_CONTENT.pt;
  if (lng.startsWith("es")) return WEEKLY_CONTENT.es;
  return WEEKLY_CONTENT.en;
}

/** The model name shown on the sample reports. */
export { SEG_MODEL as DEMO_SUMMARY_MODEL };

/** AI 设置里的 "About you / あなたについて / 关于你" 用户简介。按当前 i18n 切。 */
export function userBriefForLocale(locale: string): string {
  const lng = (locale || "").toLowerCase();
  if (lng.startsWith("ja"))
    return "フルスタックエンジニア。主に React + Rust のプロジェクトに取り組んでいます。";
  if (lng.startsWith("zh")) return "全栈开发者，主要做 React + Rust 项目。";
  return "Full-stack engineer, mostly working on React + Rust projects.";
}

/** Returns whether `text` is one of the sample briefs above, that is, the visitor kept it. */
export function isDemoUserBrief(text: string): boolean {
  return ["ja", "zh", "en"].some((lng) => userBriefForLocale(lng) === text);
}

/** 本机设备的默认名字（mockDevices 里写的是中文）。按当前 i18n 切；用户改过名就不会走到这里。 */
export function selfDeviceNameForLocale(locale: string): string {
  const lng = (locale || "").toLowerCase();
  if (lng.startsWith("zh")) return "我的工作站";
  if (lng.startsWith("ja")) return "マイワークステーション";
  if (lng.startsWith("pt")) return "Minha estação de trabalho";
  if (lng.startsWith("es")) return "Mi estación de trabajo";
  return "My workstation";
}

/** 日报那套演示配置的 id。 */
export const DEMO_PROFILE_ID = "p_demo1";

/** 日报那套演示配置在各语言里的名字。 */
const DEMO_PROFILE_NAMES = {
  zhCN: "专注工作",
  zhTW: "專注工作",
  ja: "集中作業",
  pt: "Trabalho focado",
  es: "Trabajo enfocado",
  en: "Focused work",
};

/** 日报那套演示配置的名字。按当前 i18n 切。 */
export function demoProfileNameForLocale(locale: string): string {
  const lng = (locale || "").toLowerCase();
  if (/^zh[-_]?(tw|hk|mo|hant)/.test(lng)) return DEMO_PROFILE_NAMES.zhTW;
  if (lng.startsWith("zh")) return DEMO_PROFILE_NAMES.zhCN;
  if (lng.startsWith("ja")) return DEMO_PROFILE_NAMES.ja;
  if (lng.startsWith("pt")) return DEMO_PROFILE_NAMES.pt;
  if (lng.startsWith("es")) return DEMO_PROFILE_NAMES.es;
  return DEMO_PROFILE_NAMES.en;
}

/** 名字还是某种语言的默认名，说明用户没改过。 */
export function isDemoProfileName(name: string): boolean {
  return Object.values(DEMO_PROFILE_NAMES).includes(name);
}

/** 默认（中文）。api-mock 初始化 state 时用；运行时通过 dailySegmentsForLocale 按 i18n 切。 */
export const mockDailySegments: SegmentSummaryRow[] = buildSegments("zh-CN", SEG_CONTENT_ZH);

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
  screenshotOnSwitch: true,
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
  recordBrowserHost: true,
  ignoreRules: [],
  // 演示版默认打开：图标来自 demo 自带的素材，不会联网
  downloadSiteIcons: true,
  downloadSources: { ...DEFAULT_DOWNLOAD_SOURCES },
  minimizeToTray: true,
  autoUpdateEnabled: true,
  autoUpdateInterval: "daily",
  lastUpdateCheckAt: new Date().toISOString(),
  idleThresholdSeconds: 300,
  memoryOcrResident: false,
  memoryOcrDailyAt: null,
  memoryOcrDailyTimes: [],
  chatPrivacyAcknowledged: false,
  syncAiSummaries: false,
  syncChatHistory: false,
  syncScreenMemory: false,
  ai: {
    // 让「云端 API」有几个标签可看；key 是假的，演示里的测试连接也是假的
    cloud: [
      {
        id: "c1",
        name: "openai · gpt-4o-mini",
        dialect: "openai",
        endpoint: "https://api.openai.com/v1",
        model: "gpt-4o-mini",
        apiKey: "sk-demo",
        contextLength: null,
      },
      {
        id: "c2",
        name: "deepseek-chat",
        dialect: "deepseek",
        endpoint: "https://api.deepseek.com/v1",
        model: "deepseek-chat",
        apiKey: "sk-demo",
        contextLength: null,
      },
    ],
    picks: {
      // 演示里下载好的那个模型（mockLocalModels）
      daily: { kind: "local", file: "Qwen2.5-VL-3B-Instruct-Q4_K_M.gguf" },
      weekly: { kind: "inherit" },
      chat: { kind: "local", file: "Qwen2.5-VL-3B-Instruct-Q4_K_M.gguf" },
    },
    advanced: [],
    userBrief: "全栈开发者，主要做 React + Rust 项目。",
    segments: DEFAULT_AI_SEGMENTS.map((seg) => ({ ...seg })),
    // One demo profile, so the pickers have something in them
    visibility: {
      profiles: {
        [DEMO_PROFILE_ID]: {
          name: DEMO_PROFILE_NAMES.zhCN,
          categories: ["code", "office", "workchat"],
        },
      },
      auto: { daily: null, weekly: null },
    },
    modelsPath: "C:\\Users\\demo\\AppData\\Roaming\\Hindsight\\ai\\models",
    promptLanguage: "zh",
    promptOverrides: {
      systemZh: "",
      systemEn: "",
      systemJa: "",
      systemPt: "",
      systemTw: "",
      systemEs: "",
    },
    autoSummary: true,
    autoSummaryTimes: ["23:00"],
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
  backend: "drive",
  signedIn: false,
  uid: null,
  email: null,
  configured: false,
  webdavUrl: null,
  webdavUser: null,
};

export const mockSyncStatus: SyncStatus = {
  running: false,
  syncInFlight: false,
  lastPushedAt: new Date(Date.now() - 1000 * 60 * 30).toISOString(),
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
  // 与真实的 CUDA 12.4 版估算一致（主程序加 cudart 运行库）
  estimatedBytes: 605 * 1024 * 1024,
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
    supportsTools: true,
  },
  {
    // 推荐模型的 mmproj 落盘时带主文件名前缀，同 mmprojSaveAs
    filename: "Qwen2.5-VL-3B-Instruct-Q4_K_M__mmproj-Qwen2.5-VL-3B-Instruct-f16.gguf",
    path: "C:\\Users\\demo\\AppData\\Roaming\\Hindsight\\ai\\models\\Qwen2.5-VL-3B-Instruct-Q4_K_M__mmproj-Qwen2.5-VL-3B-Instruct-f16.gguf",
    sizeBytes: 1_300_000_000,
    isMmproj: true,
    supportsTools: null,
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
    caps: ["VISION", "TEXT", "DEFAULT"],
    released: "2025-01-26",
  },
  // 下面几个照抄 src-tauri/resources/recommended-models.json，让「下载本机模型」有内容；
  // logo 用内置的品牌图，演示不连外网
  {
    displayName: "Qwen3 0.6B (Q4_K_M)",
    repo: "unsloth/Qwen3-0.6B-GGUF",
    mainFile: "Qwen3-0.6B-Q4_K_M.gguf",
    mainBytes: 396_705_472,
    mmprojFile: null,
    mmprojBytes: 0,
    logoUrl: null,
    vision: false,
    brand: "Qwen",
    caps: ["TEXT", "FAST"],
    released: "2025-04-27",
  },
  {
    displayName: "Gemma 4 E2B-it (Q4_K_M)",
    repo: "unsloth/gemma-4-E2B-it-GGUF",
    mainFile: "gemma-4-E2B-it-Q4_K_M.gguf",
    mainBytes: 3_106_736_256,
    mmprojFile: "mmproj-F16.gguf",
    mmprojBytes: 985_654_080,
    logoUrl: null,
    vision: true,
    brand: "Google",
    caps: ["VISION", "TEXT", "BALANCED"],
    released: "2026-03-02",
  },
  {
    displayName: "DeepSeek R1 Distill Qwen 7B (Q4_K_M)",
    repo: "unsloth/DeepSeek-R1-Distill-Qwen-7B-GGUF",
    mainFile: "DeepSeek-R1-Distill-Qwen-7B-Q4_K_M.gguf",
    mainBytes: 4_683_073_248,
    mmprojFile: null,
    mmprojBytes: 0,
    logoUrl: null,
    vision: false,
    brand: "DeepSeek",
    caps: ["R1", "REASONING"],
    released: "2025-01-20",
  },
  {
    displayName: "Qwen3 8B (Q4_K_M)",
    repo: "unsloth/Qwen3-8B-GGUF",
    mainFile: "Qwen3-8B-Q4_K_M.gguf",
    mainBytes: 5_027_784_512,
    mmprojFile: null,
    mmprojBytes: 0,
    logoUrl: null,
    vision: false,
    brand: "Qwen",
    caps: ["TEXT", "BALANCED"],
    released: "2025-04-27",
  },
  {
    displayName: "GLM-4.6V-Flash (Q4_K_M)",
    repo: "unsloth/GLM-4.6V-Flash-GGUF",
    mainFile: "GLM-4.6V-Flash-Q4_K_M.gguf",
    mainBytes: 6_166_578_784,
    mmprojFile: "mmproj-F16.gguf",
    mmprojBytes: 1_786_960_000,
    logoUrl: null,
    vision: true,
    brand: "Z.AI",
    caps: ["VISION", "TEXT", "FAST"],
    released: "2025-12-07",
  },
  {
    displayName: "gpt-oss 20B (Q4_K_M)",
    repo: "unsloth/gpt-oss-20b-GGUF",
    mainFile: "gpt-oss-20b-Q4_K_M.gguf",
    mainBytes: 11_624_759_488,
    mmprojFile: null,
    mmprojBytes: 0,
    logoUrl: null,
    vision: false,
    brand: "OpenAI",
    caps: ["TEXT", "REASONING"],
    released: "2025-08-04",
  },
];

// ────────────────────────────────────────────
// App groups / unclassified（Categories 页用）
// ────────────────────────────────────────────

// 真实感的应用清单 —— 让「应用」页有内容；recentSecs ≈ 近 7 天累计秒数。
// lastDeviceId 跟 mockDevices 对得上：demo-self（Win 工作站）+ demo-mac（MacBook）。
// 部分 group 有多 member（合并组，比如 Code/cursor 合并到 "Visual Studio Code"）。
const H = 3600;
export const mockAppGroups: AppGroup[] = [
  {
    id: "g-vscode",
    displayName: "Visual Studio Code",
    categoryId: "code",
    members: [
      { processName: "Code.exe", recentSecs: 18 * H, lastDeviceId: "demo-self" },
      { processName: "cursor.exe", recentSecs: 6 * H, lastDeviceId: "demo-self" },
      // 同一个应用在 MacBook 上的进程名不同，配对后跟 Windows 上的排在同一行
      { processName: "Code", recentSecs: 6 * H, lastDeviceId: "demo-mac" },
    ],
  },
  {
    id: "g-terminal",
    displayName: "Windows Terminal",
    categoryId: "code",
    members: [{ processName: "WindowsTerminal.exe", recentSecs: 4 * H, lastDeviceId: "demo-self" }],
  },
  {
    id: "g-chrome",
    displayName: "Google Chrome",
    categoryId: "browse",
    members: [
      { processName: "chrome.exe", recentSecs: 22 * H, lastDeviceId: "demo-self" },
      { processName: "Google Chrome", recentSecs: 8 * H, lastDeviceId: "demo-mac" },
    ],
  },
  {
    id: "g-firefox",
    displayName: "Firefox",
    categoryId: "browse",
    members: [{ processName: "firefox.exe", recentSecs: 3 * H, lastDeviceId: "demo-mac" }],
  },
  {
    id: "g-telegram",
    displayName: "Telegram",
    categoryId: "talk",
    members: [{ processName: "Telegram.exe", recentSecs: 2 * H + 1200, lastDeviceId: "demo-self" }],
  },
  {
    id: "g-teams",
    displayName: "Microsoft Teams",
    categoryId: "workchat",
    members: [{ processName: "Teams.exe", recentSecs: 5 * H, lastDeviceId: "demo-self" }],
  },
  {
    id: "g-obsidian",
    displayName: "Obsidian",
    categoryId: "office",
    members: [{ processName: "Obsidian.exe", recentSecs: 7 * H, lastDeviceId: "demo-mac" }],
  },
  {
    id: "g-notion",
    displayName: "Notion",
    categoryId: "office",
    members: [
      { processName: "Notion.exe", recentSecs: 3 * H, lastDeviceId: "demo-self" },
      { processName: "Notion", recentSecs: 2 * H, lastDeviceId: "demo-mac" },
    ],
  },
  {
    id: "g-spotify",
    displayName: "Spotify",
    categoryId: "video",
    members: [
      { processName: "Spotify.exe", recentSecs: 12 * H, lastDeviceId: "demo-self" },
      { processName: "Spotify", recentSecs: 3 * H, lastDeviceId: "demo-mac" },
    ],
  },
  {
    id: "g-discord",
    displayName: "Discord",
    categoryId: "talk",
    members: [{ processName: "Discord.exe", recentSecs: 4 * H, lastDeviceId: "demo-mac" }],
  },
  {
    id: "g-steam",
    displayName: "Steam",
    categoryId: "game",
    members: [{ processName: "Steam.exe", recentSecs: 1 * H + 1800, lastDeviceId: "demo-self" }],
  },
  // 未归类（categoryId: null）—— 给「未分类」筛选有内容
  {
    id: "g-discord-game",
    displayName: "Discord Overlay",
    categoryId: null,
    members: [{ processName: "discord.exe", recentSecs: 30 * 60, lastDeviceId: "demo-self" }],
  },
  {
    id: "g-github",
    displayName: "GitHub Desktop",
    categoryId: null,
    members: [{ processName: "GitHubDesktop.exe", recentSecs: 22 * 60, lastDeviceId: "demo-self" }],
  },
];

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
