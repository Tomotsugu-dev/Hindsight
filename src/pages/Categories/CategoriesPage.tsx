import { Outlet } from "react-router-dom";
import { useTranslation } from "react-i18next";
import { type TabDef } from "../../components/TabNav/TabNav";
import { FloatingTabNav } from "../../components/TabNav/FloatingTabNav";
import styles from "./Categories.module.css";

// tab 路由元数据；label 通过 t() 动态解析（同 SettingsPage 的写法）
const TABS: TabDef[] = [
  { to: "", labelKey: "categories.tabs.list", end: true },
  { to: "apps", labelKey: "categories.tabs.apps" },
];

/**
 * 分类页外壳。
 *
 * 「管理分类」和「给应用归类」本是同一件事的两面——配分类时想看有哪些应用，
 * 归应用时想看有哪些分类，拆成两个侧栏入口只会让人来回跳。这里用 tab 把它们
 * 收回一处（曾经就是这个形态，中途拆成独立 /apps 页，现在合回来）。
 */
export default function CategoriesPage() {
  const { t } = useTranslation();
  return (
    <div className={styles.page}>
      <header className={styles.header}>
        <h1 className={styles.title}>{t("categories.title")}</h1>
      </header>

      <FloatingTabNav tabs={TABS} ariaLabel={t("categories.title")} />

      <section className={styles.tabContent}>
        <Outlet />
      </section>
    </div>
  );
}
