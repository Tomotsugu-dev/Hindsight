import { getCategory } from "../../config/categories";
import type { DaySummary } from "./mockData";
import styles from "./DailyBarChart.module.css";

interface DailyBarChartProps {
  /** 按日聚合的数据 */
  days: DaySummary[];
  /** 给每一根柱子返回 X 轴标签；返回 null 则不画 */
  xLabel?: (day: DaySummary, index: number) => string | null;
}

function fmtTickLabel(min: number): string {
  if (min === 0) return "0";
  if (min % 60 === 0) return `${min / 60}h`;
  return `${min}m`;
}

/** 把 max 向上对齐到合理的整时刻度 */
function niceYMax(max: number): number {
  if (max <= 0) return 240;
  const candidates = [120, 240, 360, 480, 600, 720, 900, 1080, 1320];
  for (const c of candidates) {
    if (max <= c) return c;
  }
  return Math.ceil(max / 120) * 120;
}

export function DailyBarChart({ days, xLabel }: DailyBarChartProps) {
  const totals = days.map((d) => d.segments.reduce((s, x) => s + x.minutes, 0));
  const maxTotal = Math.max(0, ...totals);
  const yMax = niceYMax(maxTotal);

  const yTicks = [yMax / 4, yMax / 2, (3 * yMax) / 4, yMax];

  return (
    <div className={styles.chart}>
      <div className={styles.plot}>
        <div className={styles.yAxis} aria-hidden>
          {yTicks.map((t) => (
            <span
              key={t}
              className={styles.yTick}
              style={{ bottom: `${(t / yMax) * 100}%` }}
            >
              {fmtTickLabel(t)}
            </span>
          ))}
        </div>

        <div className={styles.plotArea}>
          {yTicks.map((t) => (
            <div
              key={t}
              className={styles.gridLine}
              style={{ bottom: `${(t / yMax) * 100}%` }}
              aria-hidden
            />
          ))}

          <div className={styles.bars}>
            {days.map((day, i) => {
              const total = day.segments.reduce((s, x) => s + x.minutes, 0);
              const heightPct = (total / yMax) * 100;
              return (
                <div key={i} className={styles.column}>
                  <div
                    className={styles.bar}
                    style={{ height: `${heightPct}%` }}
                    title={
                      total > 0
                        ? `${day.date.getMonth() + 1}/${day.date.getDate()} — ${fmtTickLabel(total)}`
                        : undefined
                    }
                  >
                    {day.segments.map((seg) => {
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

      <div className={styles.xAxis}>
        {days.map((day, i) => {
          const text = xLabel ? xLabel(day, i) : null;
          if (!text) return null;
          return (
            <span
              key={i}
              className={styles.xLabel}
              style={{ left: `${((i + 0.5) / days.length) * 100}%` }}
            >
              {text}
            </span>
          );
        })}
      </div>
    </div>
  );
}
