import { useTranslation } from "react-i18next";
import ListTab from "./tabs/ListTab";
import styles from "./Categories.module.css";

/**
 * 分类页：分类 CRUD（新建 / 改名 / 换色换图标 / 删除 / 拖拽排序），
 * 每条分类展开显示绑定的应用。
 *
 * 原本这里是 tab 外壳（分类 + 应用配对两个 tab），现在「应用配对」拆到独立的
 * /apps 页面（侧边栏「应用」），这里退化成单页直接渲染 ListTab。
 */
export default function CategoriesPage() {
  const { t } = useTranslation();
  return (
    <div className={styles.page}>
      <header className={styles.header}>
        <h1 className={styles.title}>{t("categories.title")}</h1>
      </header>
      <ListTab />
    </div>
  );
}
