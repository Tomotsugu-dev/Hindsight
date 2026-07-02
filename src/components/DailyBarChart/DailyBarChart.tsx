import { useTranslation } from "react-i18next";
import { useCategories } from "../../state/categories";
import { useIsDark } from "../../hooks/useTheme";
import { adjustCategoryColor } from "../../utils/categoryColor";
import { formatAxisTick, useDurationFormatter } from "../../utils/duration";
import type { DaySummary } from "../../api/hindsight";
import styles from "./DailyBarChart.module.css";

interface DailyBarChartProps {
  /** 按日聚合的数据 */
  days: DaySummary[];
  /** 给每一根柱子返回 X 轴标签；返回 null 则不画 */
  xLabel?: (day: DaySummary, index: number) => string | null;
  /** 选中的日期 index；null = 没选中。 */
  selectedIndex?: number | null;
  /** 点击触发；不传 → 柱子非交互（PeriodCard prev/next 静态副本用）。 */
  onIndexClick?: (index: number) => void;
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

export function DailyBarChart({
  days,
  xLabel,
  selectedIndex = null,
  onIndexClick,
}: DailyBarChartProps) {
  const { t } = useTranslation();
  const { getCategory } = useCategories();
  const isDark = useIsDark();
  const fmtHM = useDurationFormatter();
  const totals = days.map((d) => d.segments.reduce((s, x) => s + x.minutes, 0));
  const maxTotal = Math.max(0, ...totals);
  const yMax = niceYMax(maxTotal);

  const yTicks = [yMax / 4, yMax / 2, (3 * yMax) / 4, yMax];

  // Y 轴刻度 —— 轴刻度全语言统一英文短格式（45m / 11h / 16.5h），跟日统计一致；
  // tooltip / 页头等句子语境仍走本地化 fmtHM
  const fmtTickLabel = formatAxisTick;

  // 柱子 tooltip —— 月/日 + 时长
  const fmtBarTitle = (day: DaySummary, total: number): string =>
    t("week.chart.barTitle", {
      date: t("week.shortDate", {
        month: day.date.getMonth() + 1,
        day: day.date.getDate(),
      }),
      duration: fmtHM(total),
    });

  return (
    <div className={styles.chart}>
      <div className={styles.plot}>
        <div className={styles.yAxis} aria-hidden>
          {yTicks.map((tick) => (
            <span
              key={tick}
              className={styles.yTick}
              style={{ bottom: `${(tick / yMax) * 100}%` }}
            >
              {fmtTickLabel(tick)}
            </span>
          ))}
        </div>

        <div className={styles.plotArea}>
          {yTicks.map((tick) => (
            <div
              key={tick}
              className={styles.gridLine}
              style={{ bottom: `${(tick / yMax) * 100}%` }}
              aria-hidden
            />
          ))}

          <div className={styles.bars}>
            {days.map((day, i) => {
              const total = day.segments.reduce((s, x) => s + x.minutes, 0);
              const heightPct = (total / yMax) * 100;
              const interactive = !!onIndexClick;
              const selected = selectedIndex === i;
              const dimmed = selectedIndex !== null && selectedIndex !== i;
              return (
                <button
                  type="button"
                  key={i}
                  className={styles.column}
                  onClick={interactive ? () => onIndexClick?.(i) : undefined}
                  disabled={!interactive}
                  data-bar-button=""
                  data-selected={selected || undefined}
                  data-dimmed={dimmed || undefined}
                  aria-label={total > 0 ? fmtBarTitle(day, total) : undefined}
                >
                  <div
                    className={styles.bar}
                    style={{ height: `${heightPct}%` }}
                    data-selected={selected || undefined}
                    data-dimmed={dimmed || undefined}
                  >
                    {day.segments.map((seg) => {
                      if (total === 0) return null;
                      // categoryId 解析不到（分类刚被删、缓存未刷新）时不能跳过：
                      // 柱高按全部 segments 求和，跳过会在柱顶留透明缺口。用中性灰兜底。
                      const cat = getCategory(seg.categoryId);
                      const color = cat
                        ? adjustCategoryColor(cat.color, isDark)
                        : "var(--cat-fallback, #9ca3af)";
                      return (
                        <div
                          key={seg.categoryId}
                          className={styles.segment}
                          style={{
                            height: `${(seg.minutes / total) * 100}%`,
                            background: color,
                          }}
                        />
                      );
                    })}
                  </div>
                </button>
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
