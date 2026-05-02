import type { DaySummary, AppUsage } from "../Week/mockData";

/** 该月的 1 号 0 点 */
function firstDayOf(monthOffset: number): Date {
  const today = new Date();
  const d = new Date(today.getFullYear(), today.getMonth() + monthOffset, 1);
  d.setHours(0, 0, 0, 0);
  return d;
}

function daysInMonth(date: Date): number {
  return new Date(date.getFullYear(), date.getMonth() + 1, 0).getDate();
}

/** 给一个月生成每天的占位数据 */
export function getMonthDays(monthOffset: number): DaySummary[] {
  const first = firstDayOf(monthOffset);
  const today = new Date();
  today.setHours(23, 59, 59, 999);

  const total = daysInMonth(first);
  const result: DaySummary[] = [];

  for (let i = 0; i < total; i++) {
    const d = new Date(first);
    d.setDate(i + 1);

    if (d > today) {
      result.push({ date: d, segments: [] });
      continue;
    }

    const dow = d.getDay() === 0 ? 6 : d.getDay() - 1;
    const isWeekend = dow >= 5;
    const seed = Math.abs(monthOffset) * 31 + i;
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

export function getMonthApps(monthOffset: number): AppUsage[] {
  if (firstDayOf(monthOffset) > new Date()) return [];
  const days = getMonthDays(monthOffset);
  const total = days.reduce(
    (s, d) => s + d.segments.reduce((x, y) => x + y.minutes, 0),
    0,
  );
  const weights = [0.32, 0.21, 0.14, 0.09, 0.08];
  return BASE_APPS.map((a, i) => ({
    ...a,
    minutes: Math.round(total * weights[i]),
  }));
}

export function getMonthRange(monthOffset: number): { first: Date; last: Date } {
  const first = firstDayOf(monthOffset);
  const last = new Date(first.getFullYear(), first.getMonth() + 1, 0);
  return { first, last };
}
