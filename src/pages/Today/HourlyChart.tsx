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
  /** Y 轴最大值（分钟）。默认 60（单设备一小时）。多设备聚合时传 60 × deviceCount。*/
  maxMinutes?: number;
}

/** X 轴标签 */
const X_LABELS = [0, 6, 12, 18, 24];

function formatMinLabel(min: number): string {
  if (min < 60) return `${min}m`;
  if (min % 60 === 0) return `${min / 60}h`;
  // 半小时精度足够，避免 "1.33h" 这类
  return `${(min / 60).toFixed(1)}h`;
}

export function HourlyChart({ hours, workHours, maxMinutes = 60 }: HourlyChartProps) {
  const { getCategory } = useCategories();
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
            {hours.map((slot) => {
              const total = slot.segments.reduce((s, x) => s + x.minutes, 0);
              const heightPct = Math.min((total / maxMinutes) * 100, 100);

              return (
                <div key={slot.hour} className={styles.column}>
                  <div
                    className={styles.bar}
                    style={{ height: `${heightPct}%` }}
                    title={
                      total > 0
                        ? `${String(slot.hour).padStart(2, "0")}:00 — ${total} 分`
                        : undefined
                    }
                  >
                    {slot.segments.map((seg) => {
                      const cat = getCategory(seg.categoryId);
                      if (!cat || total === 0) return null;
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
            })}
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
