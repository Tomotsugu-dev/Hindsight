import { Outlet } from "react-router-dom";
import { useTranslation } from "react-i18next";
import { type TabDef } from "../../components/TabNav/TabNav";
import { FloatingTabNav } from "../../components/TabNav/FloatingTabNav";
import styles from "./Categories.module.css";

// tab 路由元数据；label 通过 t() 动态解析
const TABS: TabDef[] = [
  { to: "", labelKey: "categories.tabs.list", end: true },
  { to: "pairing", labelKey: "categories.tabs.pairing" },
];

/**
 * 应用分类页外壳：标题 + 2 个 tab + Outlet。
 *
 * 历史上这里是一个长滚动页（分类列表 + 多设备合并）。tab 化让两块独立的功能各自占
 * 一屏，新用户不会错过"多设备合并"那块在长页中段的存在。
 */
export default function CategoriesPage() {
  const { t } = useTranslation();
  return (
    <div className={styles.page}>
      <header className={styles.header}>
        <h1 className={styles.title}>{t("categories.title")}</h1>
      </header>

      <FloatingTabNav tabs={TABS} ariaLabel={t("categories.title")} />

      {/* Outlet 直接落进 .page 的 flex-column；.page 自带 gap: 24px 处理间距，
          不需要额外的 tabContent 包装。 */}
      <Outlet />
    </div>
  );
}
