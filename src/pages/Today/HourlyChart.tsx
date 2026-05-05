import { useMemo } from "react";
import { useCategories } from "../../state/categories";
import type { HourSlot } from "../../api/hindsight";
import styles from "./HourlyChart.module.css";

export interface WorkRange {
  startHour: number;
  endHour: number;
}

interface HourlyChartProps {
  hours: HourSlot[];
  workHours: WorkRange[] | null;
  /** Y 轴最大值（分钟）。不传则按 hours 自动算（1h 起步，超过则按 15min 一档向上）。*/
  maxMinutes?: number;
}

/** X 轴标签 */
const X_LABELS = [0, 6, 12, 18, 24];

function formatMinLabel(min: number): string {
  if (min < 60) return `${min}m`;
  if (min % 60 === 0) return `${min / 60}h`;
  // 0.25h 档位（1.25 / 1.5 / 1.75 / ...）：保留 2 位小数后用 parseFloat 去尾 0
  // 避免 toFixed(1) 把 1.25 round 成 "1.3h"
  return `${parseFloat((min / 60).toFixed(2))}h`;
}

export function HourlyChart({ hours, workHours, maxMinutes: externalMax }: HourlyChartProps) {
  const { getCategory } = useCategories();
  // 没传 maxMinutes 就按 hours 自己算：1h 起步，峰值超过就按 15min 步长向上对齐
  // —— 切日期时每个 slide 自带 maxMinutes，next-day chart 不会因为复用 current-day 的 max
  // 而造成"切换后还得再 transition 一次"的问题
  const maxMinutes = useMemo(() => {
    if (externalMax !== undefined) return externalMax;
    if (!hours.length) return 60;
    const peak = hours.reduce(
      (m, h) => Math.max(m, h.segments.reduce((s, x) => s + x.minutes, 0)),
      0,
    );
    return peak <= 60 ? 60 : Math.ceil(peak / 15) * 15;
  }, [hours, externalMax]);
  // 4 等分刻度：max/4, max/2, 3max/4, max。每条对应 25/50/75/100% 高度。
  const yTicks = [maxMinutes / 4, maxMinutes / 2, (maxMinutes * 3) / 4, maxMinutes];
  return (
    <div className={styles.chart}>
      <div className={styles.plot}>
        {/* Y 轴 */}
        <div className={styles.yAxis} aria-hidden>
          {yTicks.map((t) => (
            <span
              key={t}
              className={styles.yTick}
              style={{ bottom: `${(t / maxMinutes) * 100}%` }}
            >
              {formatMinLabel(Math.round(t))}
            </span>
          ))}
        </div>

        {/* 绘图区 */}
        <div className={styles.plotArea}>
          {/* 水平参考线 */}
          {yTicks.map((t) => (
            <div
              key={t}
              className={styles.gridLine}
              style={{ bottom: `${(t / maxMinutes) * 100}%` }}
              aria-hidden
            />
          ))}

          {/* 工作时段柔色填充 */}
          {workHours?.map((range, i) => (
            <div
              key={i}
              className={styles.workBand}
              style={{
                left: `${(range.startHour / 24) * 100}%`,
                width: `${((range.endHour - range.startHour) / 24) * 100}%`,
              }}
              aria-hidden
            />
          ))}

          {/* 24 根柱子 */}
          <div className={styles.bars}>
            {hours.map((slot) => (
              <HourBar
                key={slot.hour}
                slot={slot}
                maxMinutes={maxMinutes}
                getCategory={getCategory}
              />
            ))}
          </div>
        </div>
      </div>

      {/* X 轴 */}
      <div className={styles.xAxis}>
        {X_LABELS.map((h) => (
          <span
            key={h}
            className={styles.xLabel}
            style={{ left: `${(h / 24) * 100}%` }}
          >
            {String(h).padStart(2, "0")}
          </span>
        ))}
      </div>
    </div>
  );
}

function HourBar({
  slot,
  maxMinutes,
  getCategory,
}: {
  slot: HourSlot;
  maxMinutes: number;
  getCategory: (id: string) => { color: string } | null | undefined;
}) {
  const total = slot.segments.reduce((s, x) => s + x.minutes, 0);
  const heightPct = Math.min((total / maxMinutes) * 100, 100);
  return (
    <div className={styles.column}>
      <div
        className={styles.bar}
        style={{ height: `${heightPct}%` }}
        title={
          total > 0
            ? `${String(slot.hour).padStart(2, "0")}:00 — ${total} 分`
            : undefined
        }
      >
        {total > 0 &&
          slot.segments.map((seg) => {
            const cat = getCategory(seg.categoryId);
            if (!cat) return null;
            return (
              <div
                key={seg.categoryId}
                className={styles.segment}
                style={{
                  height: `${(seg.minutes / total) * 100}%`,
                  background: cat.color,
                }}
              />
            );
          })}
      </div>
    </div>
  );
}
