import { Outlet } from "react-router-dom";
import { useTranslation } from "react-i18next";
import { TabNav, type TabDef } from "../../components/TabNav/TabNav";
import { DebugStateProvider } from "./DebugStateContext";
import styles from "./AISummaryPage.module.css";

/** Tab 配置：分 3 组用竖线分隔，视觉上区分语义不同的 tab：
 *  - 时间维度报告：日报 / 周报 / 月报
 *  - 对话
 *  - 调试一组：跑总结的「调试」 + 它的参数面板「调试设置」（共享 DebugStateProvider state）
 */
const TAB_GROUPS: TabDef[][] = [
  [
    { to: "", labelKey: "aiSummary.tabs.daily", end: true },
    { to: "week", labelKey: "aiSummary.tabs.week" },
    { to: "month", labelKey: "aiSummary.tabs.month" },
  ],
  [{ to: "chat", labelKey: "aiSummary.tabs.chat" }],
  [
    { to: "debug", labelKey: "aiSummary.tabs.debug" },
    { to: "debug-settings", labelKey: "aiSummary.tabs.debugSettings" },
  ],
];

/**
 * AI 总结页外壳：标题 + 6 个 tab + Outlet。
 * DebugStateProvider 包 Outlet：DebugTab 和 DebugSettingsTab 共享一份调试参数 state。
 */
export default function AISummaryPage() {
  const { t } = useTranslation();

  return (
    <div className={styles.page}>
      <header className={styles.header}>
        <h1 className={styles.title}>{t("aiSummary.title")}</h1>
      </header>

      <TabNav groups={TAB_GROUPS} ariaLabel={t("aiSummary.title")} />

      <section className={styles.tabContent}>
        <DebugStateProvider>
          <Outlet />
        </DebugStateProvider>
      </section>
    </div>
  );
}
