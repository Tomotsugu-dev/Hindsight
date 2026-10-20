import { Outlet } from "react-router-dom";
import { useTranslation } from "react-i18next";
import { type TabDef } from "../../components/TabNav/TabNav";
import { PageHeader } from "../../components/PageHeader/PageHeader";
import { usePluginsSeen } from "../../state/pluginsSeen";
import styles from "./SettingsPage.module.css";

// tab 路由元数据；label 通过 t() 动态解析
const TABS: TabDef[] = [
  { to: "", labelKey: "settings.tabs.general", end: true },
  { to: "appearance", labelKey: "settings.tabs.appearance" },
  { to: "data", labelKey: "settings.tabs.data" },
  { to: "privacy", labelKey: "settings.tabs.privacy" },
  { to: "plugins", labelKey: "settings.tabs.plugins" },
  { to: "about", labelKey: "settings.tabs.about" },
];

export default function SettingsPage() {
  const { t } = useTranslation();
  const pluginsSeen = usePluginsSeen();
  const tabs = pluginsSeen
    ? TABS
    : TABS.map((tab) => (tab.to === "plugins" ? { ...tab, badgeKey: "plugins.newBadge" } : tab));

  return (
    <div className={styles.page}>
      <PageHeader title={t("settings.pageTitle")} tabs={tabs} />

      <section className={styles.tabContent}>
        <Outlet />
      </section>
    </div>
  );
}
