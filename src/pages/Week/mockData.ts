/**
 * 占位数据 — 按日聚合（每天的总分钟数 + 分类拆分）
 */

export interface DaySummary {
  date: Date;
  segments: { categoryId: string; minutes: number }[];
}

export interface AppUsage {
  process: string;
  categoryId: string;
  minutes: number;
}

/** 该周的周一 0 点（从今天向前算） */
function mondayOf(weekOffset: number): Date {
  const today = new Date();
  const dow = today.getDay() === 0 ? 6 : today.getDay() - 1; // 周一 = 0
  const monday = new Date(today);
  monday.setDate(today.getDate() - dow + weekOffset * 7);
  monday.setHours(0, 0, 0, 0);
  return monday;
}

/** 给一周生成 7 天的占位数据 */
export function getWeekDays(weekOffset: number): DaySummary[] {
  const monday = mondayOf(weekOffset);
  const today = new Date();
  today.setHours(23, 59, 59, 999);

  const result: DaySummary[] = [];
  for (let i = 0; i < 7; i++) {
    const d = new Date(monday);
    d.setDate(monday.getDate() + i);

    if (d > today) {
      result.push({ date: d, segments: [] });
      continue;
    }

    const isWeekend = i >= 5;
    // 用 weekOffset + i 做种子产生稳定但有差异的数据
    const seed = Math.abs(weekOffset) * 11 + i;
    const factor = (isWeekend ? 0.45 : 0.95) + ((seed % 5) - 2) * 0.08;

    result.push({
      date: d,
      segments: [
        { categoryId: "code",   minutes: Math.max(0, Math.round((isWeekend ?  60 : 240) * factor)) },
        { categoryId: "browse", minutes: Math.max(0, Math.round((isWeekend ?  90 : 130) * factor)) },
        { categoryId: "talk",   minutes: Math.max(0, Math.round((isWeekend ?  20 :  55) * factor)) },
        { categoryId: "design", minutes: Math.max(0, Math.round((isWeekend ?  30 :  70) * factor)) },
        { categoryId: "fun",    minutes: Math.max(0, Math.round((isWeekend ? 130 :  50) * factor)) },
      ].filter((s) => s.minutes > 0),
    });
  }
  return result;
}

const BASE_APPS: Omit<AppUsage, "minutes">[] = [
  { process: "code.exe",    categoryId: "code"   },
  { process: "chrome.exe",  categoryId: "browse" },
  { process: "Spotify.exe", categoryId: "fun"    },
  { process: "Figma.exe",   categoryId: "design" },
  { process: "WeChat.exe",  categoryId: "talk"   },
];

export function getWeekApps(weekOffset: number): AppUsage[] {
  if (mondayOf(weekOffset) > new Date()) return [];
  const days = getWeekDays(weekOffset);
  const total = days.reduce(
    (s, d) => s + d.segments.reduce((x, y) => x + y.minutes, 0),
    0,
  );
  // 按比例分给基础应用列表
  const weights = [0.32, 0.21, 0.14, 0.09, 0.08];
  return BASE_APPS.map((a, i) => ({
    ...a,
    minutes: Math.round(total * weights[i]),
  }));
}
