import { useTranslation } from "react-i18next";
import { Sparkles, Wand2 } from "lucide-react";
import styles from "./SummaryModeToggle.module.css";

export type SummaryMode = "ai" | "quick";

/** AI 总结 ⇄ 快速模板的 segmented toggle。
 *  在 DailyTab / WeeklyTab / MonthlyTab 顶部常驻；切换后由父组件按 mode 渲染不同主体。
 *  - "ai" = 调用本地/云端大模型读截图 + 段总结，依赖硬件
 *  - "quick" = 纯 SQL 聚合 + 模板填空，瞬时返回，无硬件门槛 */
export function SummaryModeToggle({
  mode,
  onChange,
}: {
  mode: SummaryMode;
  onChange: (m: SummaryMode) => void;
}) {
  const { t } = useTranslation();
  return (
    <div className={styles.toggle} role="tablist" aria-label={t("aiSummary.modeToggle.ariaLabel")}>
      <button
        type="button"
        role="tab"
        aria-selected={mode === "ai"}
        className={`${styles.opt} ${mode === "ai" ? styles.optActive : ""}`}
        onClick={() => onChange("ai")}
        title={t("aiSummary.modeToggle.aiTooltip")}
      >
        <Sparkles size={12} strokeWidth={2} />
        <span>{t("aiSummary.modeToggle.ai")}</span>
      </button>
      <button
        type="button"
        role="tab"
        aria-selected={mode === "quick"}
        className={`${styles.opt} ${mode === "quick" ? styles.optActive : ""}`}
        onClick={() => onChange("quick")}
        title={t("aiSummary.modeToggle.quickTooltip")}
      >
        <Wand2 size={12} strokeWidth={2} />
        <span>{t("aiSummary.modeToggle.quick")}</span>
      </button>
    </div>
  );
}
