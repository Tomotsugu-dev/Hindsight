import { Outlet } from "react-router-dom";
import { useTranslation } from "react-i18next";
import { type TabDef } from "../../components/TabNav/TabNav";
import { FloatingTabNav } from "../../components/TabNav/FloatingTabNav";
import styles from "./AISettingsPage.module.css";

/** 6 个子路由对应 6 个 tab：引擎 / 模型 / 常规 / 提示词 / 云端 API / 截图洞察
 *  按 LLM pipeline：运行时（引擎 + 模型 + 参数）→ 数据采样（常规）→ 指令（提示词）→ 调用（云端 API）→ 功能（洞察行为层） */
const TABS: TabDef[] = [
  { to: "", labelKey: "aiSettings.tabs.engine", end: true },
  { to: "models", labelKey: "aiSettings.tabs.models" },
  { to: "general", labelKey: "aiSettings.tabs.general" },
  { to: "prompt", labelKey: "aiSettings.tabs.prompt" },
  { to: "external", labelKey: "aiSettings.tabs.external" },
  { to: "insight", labelKey: "aiSettings.tabs.insight" },
];

/**
 * AI 设置页外壳：标题 + 5 个 tab + Outlet。
 * 跟 SettingsPage / AISummaryPage 同构（共享 components/TabNav）。
 */
export default function AISettingsPage() {
  const { t } = useTranslation();
  return (
    <div className={styles.page}>
      <header className={styles.header}>
        <h1 className={styles.title}>{t("aiSettings.title")}</h1>
      </header>

      <FloatingTabNav tabs={TABS} ariaLabel={t("aiSettings.title")} />

      <Outlet />
    </div>
  );
}
