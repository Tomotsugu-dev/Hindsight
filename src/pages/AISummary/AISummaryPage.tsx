import { Outlet } from "react-router-dom";
import { useTranslation } from "react-i18next";
import { type TabDef } from "../../components/TabNav/TabNav";
import { FloatingTabNav } from "../../components/TabNav/FloatingTabNav";
import { DebugStateProvider } from "./DebugStateContext";
import styles from "./AISummaryPage.module.css";

/** Tab 配置：分 3 组用竖线分隔，视觉上区分语义不同的 tab：
 *  - 时间维度报告：日报 / 周报 / 月报
 *  - 搜索：屏幕记忆全文检索（回顾域的点查工具，不占侧栏导航位）
 *  - 调试：跑总结 + 看结果（旧版"调试设置"已删，参数直接走 AI 设置主页）
 *  对话已提升为独立侧栏页面（/chat）。
 */
const TAB_GROUPS: TabDef[][] = [
  [
    { to: "", labelKey: "aiSummary.tabs.daily", end: true },
    { to: "week", labelKey: "aiSummary.tabs.week" },
    { to: "month", labelKey: "aiSummary.tabs.month" },
  ],
  [{ to: "search", labelKey: "aiSummary.tabs.search" }],
  [{ to: "debug", labelKey: "aiSummary.tabs.debug" }],
];

/**
 * AI 总结页外壳：标题 + 4 个 tab + Outlet。
 * DebugStateProvider 仍保留——DebugTab 自己用 debug 参数 state（之前由 settings tab 写入）。
 */
export default function AISummaryPage() {
  const { t } = useTranslation();

  return (
    <div className={styles.page}>
      <header className={styles.header}>
        <h1 className={styles.title}>{t("aiSummary.title")}</h1>
      </header>

      {/* FloatingTabNav：滚出视口顶端时让 pill 在视口顶部浮动居中显示。 */}
      <FloatingTabNav groups={TAB_GROUPS} ariaLabel={t("aiSummary.title")} />

      <section className={styles.tabContent}>
        <DebugStateProvider>
          <Outlet />
        </DebugStateProvider>
      </section>
    </div>
  );
}
