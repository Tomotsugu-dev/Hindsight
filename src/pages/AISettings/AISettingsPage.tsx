import { Outlet } from "react-router-dom";
import { useTranslation } from "react-i18next";
import { type TabDef } from "../../components/TabNav/TabNav";
import { PageHeader } from "../../components/PageHeader/PageHeader";
import styles from "./AISettingsPage.module.css";

/** Three tabs: models, general and prompt. The engine, local models and cloud API connections
 *  are in Settings → AI models. The page is not in the sidebar: the gear on the AI summary page opens it. */
const TABS: TabDef[] = [
  { to: "models", labelKey: "aiSettings.tabs.models" },
  { to: "general", labelKey: "aiSettings.tabs.general" },
  { to: "prompt", labelKey: "aiSettings.tabs.prompt" },
];

/**
 * AI 设置页外壳：标题行（右边的 tab）+ Outlet。
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
