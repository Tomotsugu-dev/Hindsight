/**
 * TimeOfDayPicker 的纯数学:点击/拖拽比例 → 整点小时,与 "HH:00" 格式化。
 * 抽离成模块以便单测(组件本体只剩 DOM 接线)。
 */

export const clampHour = (h: number): number => Math.min(23, Math.max(0, h));

/** 轨道上的水平比例(可越界)→ 0..23 整点。第 h 格覆盖 [h/24, (h+1)/24)。 */
export function hourFromRatio(ratio: number): number {
  return clampHour(Math.floor(ratio * 24));
}

export const formatHour = (h: number): string => `${String(h).padStart(2, "0")}:00`;

/** "HH:MM" → 0..23 整点(分钟位忽略;残缺/非法输入回落 0 并钳位)。 */
export function parseHour(value: string): number {
  return clampHour(parseInt(value.split(":")[0] ?? "", 10) || 0);
}

/** 多标记:离给定小时最近的标记下标(平手取靠前的,即先添加的)。 */
export function nearestIndex(hours: number[], target: number): number {
  let best = 0;
  let bestDist = Number.POSITIVE_INFINITY;
  hours.forEach((h, i) => {
    const d = Math.abs(h - target);
    if (d < bestDist) {
      bestDist = d;
      best = i;
    }
  });
  return best;
}

/** 新增时间点的落位:从 12:00 起顺时针找第一个未占用的整点;全占返回 12。 */
export function nextFreeHour(occupied: number[]): number {
  for (let step = 0; step < 24; step += 1) {
    const h = (12 + step) % 24;
    if (!occupied.includes(h)) return h;
  }
  return 12;
}
