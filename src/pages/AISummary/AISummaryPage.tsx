import { Outlet, useLocation, useNavigate } from "react-router-dom";
import { useTranslation } from "react-i18next";
import { type TabDef } from "../../components/TabNav/TabNav";
import { PageHeader } from "../../components/PageHeader/PageHeader";
import { IconButton } from "../../components/Button/IconButton";
import { AiSettingsIcon } from "../../components/icons/AiSettingsIcon";
import { ROUTES } from "../../config/nav";
import { DebugStateProvider } from "./DebugStateContext";
import styles from "./AISummaryPage.module.css";

/** Tab 配置：分 2 组用竖线分隔，视觉上区分语义不同的 tab：
 *  - 时间维度报告：日报 / 周报 / 月报
 *  - 调试：跑总结 + 看结果（旧版"调试设置"已删，参数直接走 AI 设置主页）
 *  对话已提升为独立侧栏页面（/chat），搜索搬到了全部历史（/history/search）。
 */
const TAB_GROUPS: TabDef[][] = [
  [
    { to: "", labelKey: "aiSummary.tabs.daily", end: true },
    { to: "week", labelKey: "aiSummary.tabs.week" },
    { to: "month", labelKey: "aiSummary.tabs.month" },
  ],
  [{ to: "debug", labelKey: "aiSummary.tabs.debug" }],
];

/**
 * AI 总结页外壳：标题 + 4 个 tab + 进 AI 设置的按钮 + Outlet。
 * DebugStateProvider 仍保留——DebugTab 自己用 debug 参数 state（之前由 settings tab 写入）。
 */
export default function AISummaryPage() {
  const { t } = useTranslation();
  const navigate = useNavigate();
  const { pathname } = useLocation();
  // 周报页进周报的设置，其余进日报的
  const module = pathname.startsWith(`${ROUTES.aiSummary}/week`) ? "weekly" : "daily";

  return (
    <div className={styles.page}>
      {/* tab 条在标题行右边；滚出视口顶端时在视口顶部浮动居中显示（FloatingTabNav） */}
      <PageHeader
        title={t("aiSummary.title")}
        groups={TAB_GROUPS}
        actions={
          <IconButton
            icon={AiSettingsIcon}
            label={t("aiSettings.title")}
            size="lg"
            variant="soft"
            onClick={() => navigate(`${ROUTES.aiSettings}/general?module=${module}`)}
          />
        }
      />

      <section className={styles.tabContent}>
        <DebugStateProvider>
          <Outlet />
        </DebugStateProvider>
      </section>
    </div>
  );
}
