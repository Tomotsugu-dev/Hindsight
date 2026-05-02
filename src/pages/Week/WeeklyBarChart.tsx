import { DEFAULT_CATEGORIES, getCategory } from "../../config/categories";
import type { DaySummary } from "./mockData";
import styles from "./WeeklyBarChart.module.css";

interface WeeklyBarChartProps {
  days: DaySummary[];
}

const DOW = ["周一", "周二", "周三", "周四", "周五", "周六", "周日"];

function fmtTotal(min: number): string {
  if (min === 0) return "—";
  const h = Math.floor(min / 60);
  const m = min % 60;
  if (h === 0) return `${m}m`;
  if (m === 0) return `${h}h`;
  return `${h}h ${m}m`;
}

function fmtDate(d: Date): string {
  return `${d.getMonth() + 1}/${d.getDate()}`;
}

/** 把 segments 按 DEFAULT_CATEGORIES 顺序排列，确保跨行视觉一致 */
function sortSegments(segments: DaySummary["segments"]) {
  const order = new Map(DEFAULT_CATEGORIES.map((c, i) => [c.id, i]));
  return [...segments].sort(
    (a, b) => (order.get(a.categoryId) ?? 99) - (order.get(b.categoryId) ?? 99),
  );
}

export function WeeklyBarChart({ days }: WeeklyBarChartProps) {
  const totals = days.map((d) => d.segments.reduce((s, x) => s + x.minutes, 0));
  const maxTotal = Math.max(0, ...totals);
  const today = new Date();
  today.setHours(0, 0, 0, 0);

  return (
    <div className={styles.chart}>
      {days.map((day, i) => {
        const total = totals[i];
        const widthPct = maxTotal > 0 ? (total / maxTotal) * 100 : 0;
        const isToday = day.date.toDateString() === today.toDateString();

        return (
          <div
            key={i}
            className={`${styles.row} ${isToday ? styles.rowToday : ""}`}
          >
            <div className={styles.label}>
              <span className={styles.dow}>{DOW[i]}</span>
              <span className={styles.date}>{fmtDate(day.date)}</span>
            </div>

            <div className={styles.track}>
              {total > 0 && (
                <div className={styles.bar} style={{ width: `${widthPct}%` }}>
                  {sortSegments(day.segments).map((seg) => {
                    const cat = getCategory(seg.categoryId);
                    if (!cat) return null;
                    return (
                      <div
                        key={seg.categoryId}
                        className={styles.segment}
                        style={{
                          width: `${(seg.minutes / total) * 100}%`,
                          background: cat.color,
                        }}
                        title={`${cat.name} · ${fmtTotal(seg.minutes)}`}
                      />
                    );
                  })}
                </div>
              )}
            </div>

            <div className={`${styles.total} ${total === 0 ? styles.totalEmpty : ""}`}>
              {fmtTotal(total)}
            </div>
          </div>
        );
      })}
    </div>
  );
}
