import { getCategory } from "../../config/categories";
import type { HourSlot, WorkRange } from "./mockData";
import styles from "./HourlyChart.module.css";

interface HourlyChartProps {
  hours: HourSlot[];
  workHours: WorkRange[] | null;
}

/** Y 轴刻度（分钟，从下往上） */
const Y_TICKS = [15, 30, 45, 60];
const Y_LABEL: Record<number, string> = { 15: "15m", 30: "30m", 45: "45m", 60: "1h" };

/** X 轴标签 */
const X_LABELS = [0, 6, 12, 18, 24];

export function HourlyChart({ hours, workHours }: HourlyChartProps) {
  return (
    <div className={styles.chart}>
      <div className={styles.plot}>
        {/* Y 轴 */}
        <div className={styles.yAxis} aria-hidden>
          {Y_TICKS.map((t) => (
            <span
              key={t}
              className={styles.yTick}
              style={{ bottom: `${(t / 60) * 100}%` }}
            >
              {Y_LABEL[t]}
            </span>
          ))}
        </div>

        {/* 绘图区 */}
        <div className={styles.plotArea}>
          {/* 水平参考线 */}
          {Y_TICKS.map((t) => (
            <div
              key={t}
              className={styles.gridLine}
              style={{ bottom: `${(t / 60) * 100}%` }}
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
              const heightPct = (total / 60) * 100;

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
