import type { CSSProperties } from "react";
import { useTranslation } from "react-i18next";
import { ArrowDown, ArrowUp } from "lucide-react";
import type { PeriodInsights } from "../../hooks/usePeriodInsights";
import type { BreakdownSlice } from "../../hooks/useSuperCategoryBreakdown";
import { useDurationFormatter } from "../../utils/duration";
import styles from "./InsightTiles.module.css";

interface Props {
  insights: PeriodInsights;
  scope: "today" | "week" | "month";
  /** drill 状态下整行换 accent 色（左侧色条 + 浅 tint）；null = 默认模式 */
  drilledSlice: BreakdownSlice | null;
  /** 日均分钟数；传入后第二个 tile 从"峰值"切换为"日均" */
  avgMinutes?: number;
  /** 上期日均分钟数；传入后追加第 4 个 tile 显示日均对比 */
  prevAvgMinutes?: number;
}

const DASH = "—";

/**
 * 三页 header 与 PeriodCard 之间的 3 stat tile 横排。
 * - 任一 tile 数据 null：tile 保留位置 + 显示「—」，避免 grid 抖动
 * - 三项全 null：整行不渲染（header 收回那行高度）
 * - drill 状态：每 tile 走 super-cat accent（左 3px 色条 + 4% tint）
 */
export function InsightTiles({ insights, scope, drilledSlice, avgMinutes, prevAvgMinutes }: Props) {
  const { t } = useTranslation();
  const fmtHM = useDurationFormatter();
  const { diff, peak, third } = insights;

  if (!diff && !peak && !third && avgMinutes == null) return null;

  const showAvg = avgMinutes != null;
  const showAvgVsPrev = showAvg && prevAvgMinutes != null;

  // —— A：vs 上期 ——
  let aValue: React.ReactNode;
  if (!diff) {
    aValue = <span className={styles.dash}>{DASH}</span>;
  } else if (diff.signMinutes === 0) {
    aValue = (
      <span className={styles.value}>{t(`${scope}.insights.vsPrev.flat`)}</span>
    );
  } else {
    const abs = Math.abs(diff.signMinutes);
    const isUp = diff.signMinutes > 0;
    aValue = (
      <span
        className={`${styles.value} ${isUp ? styles.up : styles.down}`}
      >
        {isUp ? (
          <ArrowUp size={14} strokeWidth={2.4} aria-hidden />
        ) : (
          <ArrowDown size={14} strokeWidth={2.4} aria-hidden />
        )}
        {fmtHM(abs)}
      </span>
    );
  }

  // —— B：峰值 / 日均 ——
  const bValue =
    avgMinutes != null ? (
      <span className={styles.value}>{fmtHM(avgMinutes)}</span>
    ) : peak ? (
      <span className={styles.value}>{peak.label}</span>
    ) : (
      <span className={styles.dash}>{DASH}</span>
    );

  const bLabel =
    avgMinutes != null
      ? t(`${scope}.insights.avg`)
      : peak
        ? `${t(`${scope}.insights.peak`)} · ${fmtHM(peak.minutes)}`
        : t(`${scope}.insights.peak`);

  // —— C：主力 / 构成 ——
  const cValue = third ? (
    <span className={styles.value}>
      <span
        className={styles.swatch}
        style={{ background: third.color }}
        aria-hidden
      />
      {third.name} {third.pct}%
    </span>
  ) : (
    <span className={styles.dash}>{DASH}</span>
  );

  const cLabel =
    third?.kind === "composition"
      ? t(`${scope}.insights.composition`)
      : t(`${scope}.insights.dominant`);

  // —— D：日均对比 ——
  let dValue: React.ReactNode = null;
  if (showAvgVsPrev && avgMinutes != null && prevAvgMinutes != null) {
    const diffAvg = avgMinutes - prevAvgMinutes;
    if (diffAvg === 0) {
      dValue = <span className={styles.value}>{t(`${scope}.insights.vsPrev.flat`)}</span>;
    } else {
      const abs = Math.abs(diffAvg);
      const isUp = diffAvg > 0;
      dValue = (
        <span className={`${styles.value} ${isUp ? styles.up : styles.down}`}>
          {isUp ? (
            <ArrowUp size={14} strokeWidth={2.4} aria-hidden />
          ) : (
            <ArrowDown size={14} strokeWidth={2.4} aria-hidden />
          )}
          {fmtHM(abs)}
        </span>
      );
    }
  }

  const rootStyle: CSSProperties | undefined = drilledSlice
    ? ({ "--accent": drilledSlice.color } as CSSProperties)
    : undefined;

  return (
    <div
      className={`${styles.tiles} ${showAvg ? styles.hasAvg : ""}`}
      data-drill={drilledSlice ? "" : undefined}
      style={rootStyle}
    >
      <div className={styles.tile}>
        {aValue}
        <div className={styles.label}>{t(`${scope}.insights.vsPrev.label`)}</div>
      </div>
      <div className={styles.tile}>
        {bValue}
        <div className={styles.label}>{bLabel}</div>
      </div>
      {showAvgVsPrev && (
        <div className={styles.tile}>
          {dValue}
          <div className={styles.label}>{t(`${scope}.insights.avgVsPrev`)}</div>
        </div>
      )}
      <div className={styles.tile}>
        {cValue}
        <div className={styles.label}>{cLabel}</div>
      </div>
    </div>
  );
}
