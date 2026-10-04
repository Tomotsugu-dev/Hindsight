/** 一段日期，两端都含，"YYYY-MM-DD" */
export interface DayRange {
  from: string;
  to: string;
}

/** range 和 bounds 的交集；不相交时返回 null。 */
export function clipRange(range: DayRange, bounds: DayRange): DayRange | null {
  const from = range.from > bounds.from ? range.from : bounds.from;
  const to = range.to < bounds.to ? range.to : bounds.to;
  return from <= to ? { from, to } : null;
}

/** 本地日期 → "YYYY-MM-DD"，跟后端的日期键同一格式。 */
export function dayKey(date: Date): string {
  const m = String(date.getMonth() + 1).padStart(2, "0");
  const d = String(date.getDate()).padStart(2, "0");
  return `${date.getFullYear()}-${m}-${d}`;
}

/** "YYYY-MM-DD" → 那天的本地零点。 */
export function parseDayKey(key: string): Date {
  const [y, m, d] = key.split("-").map((s) => parseInt(s, 10));
  return new Date(y, m - 1, d);
}

/** 1–4 档混入主色的比例；0 档用中性灰，不混色 */
export const LEVEL_MIX = [0, 30, 50, 75, 100];

export interface HeatCell {
  key: string;
  secs: number;
  /** 0 = 没有时间；1–4 按这段日期里最忙那天的比例分档 */
  level: number;
  /** 在有记录的范围之外（第一条记录之前、今天之后），画成空格子、不能点 */
  outside: boolean;
}

/** 每行或每列是一周，从周一到周日；不属于这段日期的格子是 null */
export type HeatWeeks = (HeatCell | null)[][];

export interface HeatGrid {
  weeks: HeatWeeks;
  /** 列头的月份写在哪一列；date 是那个月里的任意一天，只用来取年和月 */
  monthCols: { date: Date; col: number }[];
}

/**
 * 把 first..last 这几天排成按周分组的格子，并按其中最忙那天分档。
 * - `secsByDay`：日期键 → 秒数，没有的天按 0 算。
 * - `from` / `to`：有记录的第一天和今天，范围外的格子标成 outside。
 */
function buildWeeks(
  first: Date,
  last: Date,
  secsByDay: Map<string, number>,
  from: string,
  to: string,
): HeatWeeks {
  const cells: HeatCell[] = [];
  // 逐日 new Date(y, m, d + i) 而不是加 24 小时：夏令时那天不是 24 小时
  for (let i = 0; ; i++) {
    const date = new Date(first.getFullYear(), first.getMonth(), first.getDate() + i);
    if (date > last) break;
    const key = dayKey(date);
    const outside = key < from || key > to;
    cells.push({ key, secs: outside ? 0 : (secsByDay.get(key) ?? 0), level: 0, outside });
  }

  const max = Math.max(...cells.map((c) => c.secs), 0);
  for (const c of cells) {
    c.level = c.secs > 0 ? Math.min(4, Math.ceil((c.secs / max) * 4)) : 0;
  }

  // 第一天前面空出几格：周一 = 0 … 周日 = 6
  const lead = (first.getDay() + 6) % 7;
  const weeks: HeatWeeks = [];
  cells.forEach((cell, i) => {
    const slot = lead + i;
    const week = Math.floor(slot / 7);
    if (!weeks[week]) weeks[week] = Array<HeatCell | null>(7).fill(null);
    weeks[week][slot % 7] = cell;
  });
  return weeks;
}

/**
 * 热力图格子：每列一周，画 `range` 这几天（一整年，或者拖出来的一段）。
 * 列头在每个月 1 号那列写月份；第一列不是 1 号开头时也写上它的月份，
 * 除非下个月的 1 号就在旁边两列里，两个字会挤在一起。
 */
export function buildHeatmap(
  range: DayRange,
  secsByDay: Map<string, number>,
  from: string,
  to: string,
): HeatGrid {
  const first = parseDayKey(range.from);
  const weeks = buildWeeks(first, parseDayKey(range.to), secsByDay, from, to);
  const monthCols: { date: Date; col: number }[] = [];
  weeks.forEach((week, col) => {
    for (const cell of week) {
      const date = cell && parseDayKey(cell.key);
      if (date && date.getDate() === 1) monthCols.push({ date, col });
    }
  });
  if (first.getDate() !== 1 && !monthCols.some((m) => m.col <= 2)) {
    monthCols.unshift({ date: first, col: 0 });
  }
  return { weeks, monthCols };
}

/** 一个月的日历格子：每行一周。`month` 是 0–11。 */
export function buildHeatMonth(
  year: number,
  month: number,
  secsByDay: Map<string, number>,
  from: string,
  to: string,
): HeatWeeks {
  return buildWeeks(new Date(year, month, 1), new Date(year, month + 1, 0), secsByDay, from, to);
}
