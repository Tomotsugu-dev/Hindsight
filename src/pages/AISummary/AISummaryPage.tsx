import { Outlet } from "react-router-dom";
import { useTranslation } from "react-i18next";
import { TabNav, type TabDef } from "../../components/TabNav/TabNav";
import styles from "./AISummaryPage.module.css";

/** Tab 配置：5 个子路由对应 5 个 tab */
const TABS: TabDef[] = [
  { to: "", labelKey: "aiSummary.tabs.daily", end: true },
  { to: "week", labelKey: "aiSummary.tabs.week" },
  { to: "month", labelKey: "aiSummary.tabs.month" },
  { to: "chat", labelKey: "aiSummary.tabs.chat" },
  { to: "debug", labelKey: "aiSummary.tabs.debug" },
];

/**
 * AI 总结页外壳：标题 + 5 个 tab + Outlet。
 * 子路由各自实现内容（DailyTab 是真主体，其它是占位）。
 */
export default function AISummaryPage() {
  const { t } = useTranslation();

  return (
    <div className={styles.page}>
      <header className={styles.header}>
        <h1 className={styles.title}>{t("aiSummary.title")}</h1>
      </header>

      <TabNav tabs={TABS} ariaLabel={t("aiSummary.title")} />

      <section className={styles.tabContent}>
        <Outlet />
      </section>
    </div>
  );
}
