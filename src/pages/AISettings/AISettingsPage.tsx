import { Outlet } from "react-router-dom";
import { useTranslation } from "react-i18next";
import { TabNav, type TabDef } from "../../components/TabNav/TabNav";
import styles from "./AISettingsPage.module.css";

/** 4 个子路由对应 4 个 tab：引擎 / 常规 / 提示词 / 云端 API
 *  按 LLM pipeline：运行时环境（引擎含模型 + 参数）→ 数据采样（常规）→ 指令（提示词）→ 调用（云端 API） */
const TABS: TabDef[] = [
  { to: "", labelKey: "aiSettings.tabs.engine", end: true },
  { to: "general", labelKey: "aiSettings.tabs.general" },
  { to: "prompt", labelKey: "aiSettings.tabs.prompt" },
  { to: "external", labelKey: "aiSettings.tabs.external" },
];

/**
 * AI 设置页外壳：标题 + 4 个 tab + Outlet。
 * 跟 SettingsPage / AISummaryPage 同构（共享 components/TabNav）。
 */
export default function AISettingsPage() {
  const { t } = useTranslation();
  return (
    <div className={styles.page}>
      <header className={styles.header}>
        <h1 className={styles.title}>{t("aiSettings.title")}</h1>
      </header>

      <TabNav tabs={TABS} ariaLabel={t("aiSettings.title")} />

      <Outlet />
    </div>
  );
}
