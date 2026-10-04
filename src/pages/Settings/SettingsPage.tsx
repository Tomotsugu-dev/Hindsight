import { Outlet } from "react-router-dom";
import { useTranslation } from "react-i18next";
import { type TabDef } from "../../components/TabNav/TabNav";
import { PageHeader } from "../../components/PageHeader/PageHeader";
import styles from "./SettingsPage.module.css";

// tab 路由元数据；label 通过 t() 动态解析
const TABS: TabDef[] = [
  { to: "", labelKey: "settings.tabs.general", end: true },
  { to: "appearance", labelKey: "settings.tabs.appearance" },
  { to: "data", labelKey: "settings.tabs.data" },
  { to: "privacy", labelKey: "settings.tabs.privacy" },
  { to: "about", labelKey: "settings.tabs.about" },
];

export default function SettingsPage() {
  const { t } = useTranslation();

  return (
    <div className={styles.page}>
      <PageHeader title={t("settings.pageTitle")} tabs={TABS} />

      <section className={styles.tabContent}>
        <Outlet />
      </section>
    </div>
  );
}
