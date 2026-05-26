import { useEffect } from "react";
import { useTranslation } from "react-i18next";
import { Plus } from "lucide-react";
import { useCategories } from "../../../state/categories";
import { useSuperCategories } from "../../../state/superCategories";
import {
  CATEGORY_PALETTE,
  ICON_NAMES,
} from "../../../config/categoryIcons";
import { SuperCategoriesTable } from "../SuperCategoriesTable";
import styles from "../Categories.module.css";

const DEFAULT_NEW_ICON = "Tag";
const SUPER_ICONS = ["Folder", "Briefcase", "Gamepad2", "BookOpen", "Coffee"];

function pickRandom<T>(arr: T[]): T {
  return arr[Math.floor(Math.random() * arr.length)];
}
/** 给「新分类」按钮挑个不撞色板首位的颜色（避免连续新建同色） */
function pickCategoryColor() {
  return pickRandom(CATEGORY_PALETTE.slice(0, CATEGORY_PALETTE.length - 2)); // 排除最后两个灰色
}

/**
 * 「分类」页：大类容器 + 子分类拖拽归类。
 *
 * v28 重构：从平铺 CRUD 列表 → 嵌套 "大类 → 分类" 双层结构。子分类拖拽到任意大类
 * label 即归入；拖到"未归入"行 = 解除归属。具体拖拽 / 改外观 / 改名 / 删除
 * 等交互都在 [`SuperCategoriesTable`] 内部。
 */
export default function ListTab() {
  const { t } = useTranslation();
  const {
    categories,
    loading,
    refresh: refreshCategories,
    create: createCategory,
  } = useCategories();
  const { create: createSuper } = useSuperCategories();

  // 每次切回本 tab 强制 refetch ——
  // CategoriesProvider 全局 mount 一次后不会自动重拉，capture 写入 app_group_members 后
  // 用户切别的 tab 再回来如果不刷新，分类卡片就一直显示老的 app 列表。
  useEffect(() => {
    void refreshCategories();
  }, [refreshCategories]);

  const handleNewCategory = async () => {
    // 新分类用随机色 + 默认 Tag icon，立刻落到「未归入」行；用户在 chip 上双击改名 / 点 icon 改外观
    await createCategory({
      name: t("categories.newCategoryDefaultName"),
      color: pickCategoryColor(),
      icon: DEFAULT_NEW_ICON,
    });
  };

  const handleNewSuper = async () => {
    // 新大类同理：随机色 + 随机 icon，立刻插入到表格末尾
    await createSuper({
      name: t("categories.super.newDefaultName"),
      color: pickCategoryColor(),
      icon: pickRandom(SUPER_ICONS.filter((n) => ICON_NAMES.includes(n))),
    });
  };

  return (
    <>
      <header className={styles.header}>
        <p className={styles.meta}>{t("categories.intro")}</p>
        {/* 两个按钮包到 .headerActions 里——避免两个 createBtn 各自 margin-left:auto
            导致 flex 把剩余空间平分给两边、按钮之间出现一大段空隙 */}
        <div className={styles.headerActions}>
          <button
            type="button"
            className={styles.createBtn}
            onClick={() => void handleNewCategory()}
          >
            <Plus size={14} strokeWidth={2} />
            {t("categories.newCategory")}
          </button>
          <button
            type="button"
            className={styles.createBtn}
            onClick={() => void handleNewSuper()}
          >
            <Plus size={14} strokeWidth={2} />
            {t("categories.super.newButton")}
          </button>
        </div>
      </header>

      {loading && categories.length === 0 ? (
        <div className={styles.empty}>{t("categories.loading")}</div>
      ) : (
        <SuperCategoriesTable />
      )}
    </>
  );
}
