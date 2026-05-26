import { useTranslation } from "react-i18next";
import { BarChart3, PieChart } from "lucide-react";
import styles from "./ViewToggle.module.css";

export type StatsView = "bars" | "pie";

interface Props {
  view: StatsView;
  onChange: (next: StatsView) => void;
}

/**
 * 「时段 / 占比」segmented：PeriodCard headLeftExtras 用。26px 高、白 thumb 滑动。
 * thumb 用 `data-view` 控制 `transform: translateX(...)`，跟 sidebar 那套同款。
 */
export function ViewToggle({ view, onChange }: Props) {
  const { t } = useTranslation();
  return (
    <div className={styles.toggle} data-view={view} role="tablist">
      <span className={styles.thumb} aria-hidden />
      <button
        type="button"
        role="tab"
        aria-selected={view === "bars"}
        aria-label={t("today.chart.viewToggle.ariaBars")}
        className={`${styles.btn} ${view === "bars" ? styles.btnActive : ""}`}
        onClick={() => onChange("bars")}
      >
        <BarChart3 size={12} strokeWidth={2.2} />
        <span>{t("today.chart.viewToggle.bars")}</span>
      </button>
      <button
        type="button"
        role="tab"
        aria-selected={view === "pie"}
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
