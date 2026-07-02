import { useMemo } from "react";
import { useTranslation } from "react-i18next";
import { useCategories } from "../../state/categories";
import { useIsDark } from "../../hooks/useTheme";
import { adjustCategoryColor } from "../../utils/categoryColor";
import { formatAxisTick } from "../../utils/duration";
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
  /** 当前选中的小时；null = 没选中（全部柱子正常态）。 */
  selectedHour?: number | null;
  /** 点击柱子触发；不传 → 柱子非交互（用于 PeriodCard 的 prev/next 静态副本）。 */
  onHourClick?: (hour: number) => void;
}

/** X 轴标签 */
const X_LABELS = [0, 6, 12, 18, 24];

// 刻度格式化抽到 utils/duration.ts 的 formatAxisTick，跟月统计 Y 轴共用同一套英文短格式
const formatMinLabel = formatAxisTick;

export function HourlyChart({
  hours,
  workHours,
  maxMinutes: externalMax,
  selectedHour = null,
  onHourClick,
}: HourlyChartProps) {
  const { t } = useTranslation();
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
                t={t}
                selected={selectedHour === slot.hour}
                dimmed={selectedHour !== null && selectedHour !== slot.hour}
                onClick={onHourClick}
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
  t,
  selected,
  dimmed,
  onClick,
}: {
  slot: HourSlot;
  maxMinutes: number;
  getCategory: (id: string) => { color: string } | null | undefined;
  t: (key: string, options?: Record<string, unknown>) => string;
  selected: boolean;
  dimmed: boolean;
  onClick?: (hour: number) => void;
}) {
  const total = slot.segments.reduce((s, x) => s + x.minutes, 0);
  const heightPct = Math.min((total / maxMinutes) * 100, 100);
  const interactive = !!onClick;
  const isDark = useIsDark();
  // 整列都可点（不只柱子的高度内），点空槽位也能选中那个小时——和"hit area
  // 限制在 bar 高度内会让点低柱难"的常见 UX 痛点对齐
  return (
    <button
      type="button"
      className={styles.column}
      onClick={interactive ? () => onClick?.(slot.hour) : undefined}
      disabled={!interactive}
      data-bar-button=""
      data-selected={selected || undefined}
      data-dimmed={dimmed || undefined}
      aria-label={t("today.chart.barTitle", {
        hour: String(slot.hour).padStart(2, "0"),
        minutes: total,
      })}
    >
      <div
        className={styles.bar}
        style={{ height: `${heightPct}%` }}
        data-selected={selected || undefined}
        data-dimmed={dimmed || undefined}
      >
        {total > 0 &&
          slot.segments.map((seg) => {
            // 分类解析不到（刚被删、缓存未刷新）不能跳过：柱高按全部 segments
            // 求和，跳过会留透明缺口（同 DailyBarChart 的兜底）。
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
}
