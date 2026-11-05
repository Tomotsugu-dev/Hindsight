// 日报、周报页面上的日期和文字格式。

/** 本地日期转 "YYYY-MM-DD"。 */
export function toDateStr(d: Date): string {
  const y = d.getFullYear();
  const m = String(d.getMonth() + 1).padStart(2, "0");
  const day = String(d.getDate()).padStart(2, "0");
  return `${y}-${m}-${day}`;
}

/** "YYYY-MM-DD" 转本地那天的 0 点。 */
export function parseDateStr(s: string): Date {
  return new Date(`${s}T00:00:00`);
}

/** 本地模型存的是文件名，去掉扩展名再显示；云端模型名原样。 */
export function modelLabel(model: string): string {
  return model.replace(/\.gguf$/i, "");
}

/**
 * 生成时间。给了 `sameDayAs` 且生成时间就在那天时只写时分，否则带上月日。
 * 存的时间读不出来时返回 null。
 */
export function formatGeneratedAt(iso: string, locale: string, sameDayAs?: string): string | null {
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return null;
  const timeOnly = sameDayAs !== undefined && toDateStr(d) === sameDayAs;
  return new Intl.DateTimeFormat(
    locale,
    timeOnly
      ? { hour: "2-digit", minute: "2-digit" }
      : { month: "numeric", day: "numeric", hour: "2-digit", minute: "2-digit" },
  ).format(d);
}

/** 整点转 "HH:00"，24 点写成 "24:00"。 */
export function hourLabel(hour: number): string {
  return `${String(hour).padStart(2, "0")}:00`;
}

/** 格子里放得下的短时长：45m、6h、6h 20m。各语言一样，和图表刻度的写法一致。 */
export function compactDuration(minutes: number): string {
  if (minutes < 60) return `${minutes}m`;
  const h = Math.floor(minutes / 60);
  const m = minutes % 60;
  return m === 0 ? `${h}h` : `${h}h ${m}m`;
}

/** 热力图分级时，一周里最忙的那天至少按这么多分钟算。 */
const HEAT_MIN_SCALE = 240;

/**
 * 热力图的深浅，1 到 5 级；0 分钟是 0 级。
 * 按这周最忙的那天分级：一天用电脑七八个小时很常见，按绝对时长会整周一个颜色。
 * 最忙的那天不到 4 小时时按 4 小时算，免得很闲的一周也被染成最深。
 */
export function heatLevel(minutes: number, weekMax: number): number {
  if (minutes <= 0) return 0;
  const scale = Math.max(weekMax, HEAT_MIN_SCALE);
  return Math.min(5, Math.max(1, Math.round((5 * minutes) / scale)));
}

/**
 * `date` 离今天往前几天：今天是 0，昨天是 -1。
 * 读不出来、或者在今天之后，都当今天。
 */
export function dayOffsetOf(date: string | null, now: Date = new Date()): number {
  if (!date) return 0;
  const target = parseDateStr(date);
  if (Number.isNaN(target.getTime())) return 0;
  const today = parseDateStr(toDateStr(now));
  // 四舍五入：夏令时那天差一个小时，不能算成差一天
  const diff = Math.round((target.getTime() - today.getTime()) / 86_400_000);
  return Math.min(diff, 0);
}

/** 周报七天格子里一天的样子。 */
export type WeekDayKind = "daily" | "noDaily" | "future";

/** 今天之后的算未来；其余看有没有日报。 */
export function weekDayKind(day: { date: string; hasDaily: boolean }, today: string): WeekDayKind {
  if (day.date > today) return "future";
  return day.hasDaily ? "daily" : "noDaily";
}
