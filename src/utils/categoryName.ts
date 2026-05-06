import type { TFunction } from "i18next";
import type { Category } from "../api/hindsight";

// 数据库 migration v5 + v10 写入的原始默认分类映射
// 仅当 id 与 name 都精确匹配时才翻译；用户改过名的不翻译
const DEFAULT_CATEGORY_NAMES: Record<string, string> = {
  code: "编程",
  browse: "浏览",
  talk: "社交",
  design: "设计",
  fun: "娱乐",
  other: "其他",
};

/** 渲染分类名：默认分类（且未被改名）走 i18n，其余直接用 category.name */
export function displayCategoryName(
  category: Pick<Category, "id" | "name">,
  t: TFunction,
): string {
  const original = DEFAULT_CATEGORY_NAMES[category.id];
  if (original !== undefined && category.name === original) {
    return t(`categories.defaults.${category.id}`);
  }
  return category.name;
}
