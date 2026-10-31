import { useTranslation } from "react-i18next";
import { useCategories } from "../../state/categories";
import { useBarParts } from "../../hooks/useBarParts";
import type { BarLevel } from "../../state/statsView";
import type { DaySummary } from "../../api/hindsight";
import { barMinutes, barSecs } from "../../lib/segments";
import styles from "./WeeklyBarChart.module.css";

interface WeeklyBarChartProps {
  days: DaySummary[];
  /** 选中的日期 index（0..days.length-1）；null = 没选中。 */
  selectedIndex?: number | null;
  /** 点击触发；不传 → 行非交互（PeriodCard prev/next 静态副本用）。 */
  onIndexClick?: (index: number) => void;
  /** Colors the bars by category or by super category. */
  level?: BarLevel;
}

// 周几标签的资源 key（按周一到周日顺序）
const DOW_KEYS = [
  "week.dow.mon",
  "week.dow.tue",
  "week.dow.wed",
  "week.dow.thu",
  "week.dow.fri",
  "week.dow.sat",
  "week.dow.sun",
] as const;

export function WeeklyBarChart({
  days,
  selectedIndex = null,
  onIndexClick,
  level = "cats",
}: WeeklyBarChartProps) {
  const { t } = useTranslation();
  const { categories } = useCategories();
  const toParts = useBarParts(level);
  const order = new Map(categories.map((c, i) => [c.id, i]));
  const sortSegments = (segments: DaySummary["segments"]) =>
    [...segments].sort(
      (a, b) => (order.get(a.categoryId) ?? 99) - (order.get(b.categoryId) ?? 99),
    );

  // 时长格式化 —— 复用 common.duration.* 资源；空值用占位符
  const fmtTotal = (min: number): string => {
    if (min === 0) return "—";
    const h = Math.floor(min / 60);
    const m = min % 60;
    if (h === 0) return t("common.duration.tickMinutes", { count: m });
    if (m === 0) return t("common.duration.tickHours", { count: h });
    return t("common.duration.hoursAndMinutesShort", { hours: h, minutes: m });
  };

  // 月/日 短日期
  const fmtDate = (d: Date): string =>
    t("week.shortDate", { month: d.getMonth() + 1, day: d.getDate() });

  const totals = days.map((d) => barMinutes(d.segments));
  const maxTotal = Math.max(0, ...totals);
  const today = new Date();
  today.setHours(0, 0, 0, 0);

  const interactive = !!onIndexClick;
  return (
    <div className={styles.chart} data-part="chart">
      {days.map((day, i) => {
        const total = totals[i];
        const totalSecs = barSecs(day.segments);
        const widthPct = maxTotal > 0 ? (total / maxTotal) * 100 : 0;
        const isToday = day.date.toDateString() === today.toDateString();
        const selected = selectedIndex === i;
        const dimmed = selectedIndex !== null && selectedIndex !== i;

        return (
          <button
            type="button"
            key={i}
            className={`${styles.row} ${isToday ? styles.rowToday : ""}`}
            onClick={interactive ? () => onIndexClick?.(i) : undefined}
            disabled={!interactive}
            data-bar-button=""
            data-selected={selected || undefined}
            data-dimmed={dimmed || undefined}
          >
            <div className={styles.label}>
              <span className={styles.dow}>{t(DOW_KEYS[i])}</span>
              <span className={styles.date}>{fmtDate(day.date)}</span>
            </div>

            <div className={styles.track}>
              {total > 0 && (
                <div className={styles.bar} style={{ width: `${widthPct}%` }}>
                  {toParts(sortSegments(day.segments)).map((part) => {
                    const minutes = fmtTotal(Math.round(part.secs / 60));
                    return (
                      <div
                        key={part.id}
                        className={styles.segment}
                        style={{
                          width: `${(part.secs / totalSecs) * 100}%`,
                          background: part.color,
                        }}
                        title={part.name ? `${part.name} ·${minutes}` : minutes}
                      />
                    );
                  })}
                </div>
              )}
            </div>

            <div className={`${styles.total} ${total === 0 ? styles.totalEmpty : ""}`}>
              {fmtTotal(total)}
            </div>
          </button>
        );
      })}
    </div>
  );
}
