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
