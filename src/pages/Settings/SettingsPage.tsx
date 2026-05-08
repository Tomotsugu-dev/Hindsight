import { Outlet } from "react-router-dom";
import { useTranslation } from "react-i18next";
import { type TabDef } from "../../components/TabNav/TabNav";
import { FloatingTabNav } from "../../components/TabNav/FloatingTabNav";
import styles from "./SettingsPage.module.css";

// tab 路由元数据；label 通过 t() 动态解析
const TABS: TabDef[] = [
  { to: "", labelKey: "settings.tabs.general", end: true },
  { to: "data", labelKey: "settings.tabs.data" },
  { to: "privacy", labelKey: "settings.tabs.privacy" },
  { to: "about", labelKey: "settings.tabs.about" },
];

export default function SettingsPage() {
  const { t } = useTranslation();

  return (
    <div className={styles.page}>
      <header className={styles.header}>
        <h1 className={styles.title}>{t("settings.pageTitle")}</h1>
      </header>

      <FloatingTabNav tabs={TABS} ariaLabel={t("settings.pageTitle")} />

      <section className={styles.tabContent}>
        <Outlet />
      </section>
    </div>
  );
}
