import { useState } from "react";
import { useTranslation } from "react-i18next";
import { PlaceholderTab } from "./PlaceholderTab";
import { QuickSummaryView } from "./QuickSummaryView";
import { SummaryModeToggle, type SummaryMode } from "../components/SummaryModeToggle";

/** 月报 tab：AI 月报功能未实现，所以默认就开"快速模板"。
 *  保留模式切换让用户体感跟 daily/weekly 一致（切到 AI 也能看到"开发中"占位）。 */
export default function MonthlyTab() {
  const { t } = useTranslation();
  const [mode, setMode] = useState<SummaryMode>("quick");

  return (
    <>
      <SummaryModeToggle mode={mode} onChange={setMode} />
      {mode === "quick" ? (
        <QuickSummaryView scope="month" />
      ) : (
        <PlaceholderTab title={t("aiSummary.monthly.title")} hint={t("aiSummary.monthly.hint")} />
      )}
    </>
  );
}
