import { useTranslation } from "react-i18next";
import { BarChart3, PieChart } from "lucide-react";
import styles from "./ViewToggle.module.css";
import type { StatsView } from "../../state/statsView";

// 类型真源在 state/statsView.ts；这里再导出，消费方一处 import 拿组件 + 类型。
export type { StatsView };

interface Props {
  view: StatsView;
  /** What one bar stands for in the bar view: an hour (daily stats) or a day (week, month). */
  bar: "hour" | "day";
  onChange: (next: StatsView) => void;
}

/**
 * 「时段 / 占比」underline tab：PeriodCard headLeftExtras 用。
 * 纯文字 + icon，下方 2px 紫色 underline 跟 `data-view` 滑。
 */
export function ViewToggle({ view, bar, onChange }: Props) {
  const { t } = useTranslation();
  const byDay = bar === "day";
  return (
    <div className={styles.toggle} data-view={view}>
      <span className={styles.underline} aria-hidden />
      <button
        type="button"
        aria-pressed={view === "bars"}
        aria-label={t(
          byDay ? "today.chart.viewToggle.ariaDays" : "today.chart.viewToggle.ariaBars",
        )}
        className={`${styles.btn} ${view === "bars" ? styles.btnActive : ""}`}
        onClick={() => onChange("bars")}
      >
        <BarChart3 size={12} strokeWidth={2.2} />
        <span>{t(byDay ? "today.chart.viewToggle.days" : "today.chart.viewToggle.bars")}</span>
      </button>
      <button
        type="button"
        aria-pressed={view === "pie"}
        aria-label={t("today.chart.viewToggle.ariaPie")}
        className={`${styles.btn} ${view === "pie" ? styles.btnActive : ""}`}
        onClick={() => onChange("pie")}
      >
        <PieChart size={12} strokeWidth={2.2} />
        <span>{t("today.chart.viewToggle.pie")}</span>
      </button>
    </div>
  );
}
