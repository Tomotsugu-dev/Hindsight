import { Outlet } from "react-router-dom";
import { useTranslation } from "react-i18next";
import { type TabDef } from "../../components/TabNav/TabNav";
import { PageHeader } from "../../components/PageHeader/PageHeader";
import styles from "./AISettingsPage.module.css";

/** 4 个子路由对应 4 个 tab：模型 / 常规 / 提示词 / 云端 API。引擎在 设置 → AI 引擎。
 *  页面不在侧栏里，从各页面标题行的齿轮进来。 */
const TABS: TabDef[] = [
  { to: "models", labelKey: "aiSettings.tabs.models" },
  { to: "general", labelKey: "aiSettings.tabs.general" },
  { to: "prompt", labelKey: "aiSettings.tabs.prompt" },
  { to: "external", labelKey: "aiSettings.tabs.external" },
];

/**
 * AI 设置页外壳：标题行（右边 4 个 tab）+ Outlet。
 * 跟 SettingsPage / AISummaryPage 同构（共享 components/PageHeader）。
 */
export default function AISettingsPage() {
  const { t } = useTranslation();
  return (
    <div className={styles.page}>
      <PageHeader title={t("aiSettings.title")} tabs={TABS} />

      <Outlet />
    </div>
  );
}
