import type { TFunction } from "i18next";
import type { Category, SuperCategory } from "../api/hindsight";

// 数据库 migration v5 + v10 + v27 + v29 + v30 + v31 写入的原始默认分类映射
// 仅当 id 与 name 都精确匹配时才翻译；用户改过名的不翻译
// 注：v31 把 `fun` 软删 + 拆成 `game` + `video`，但保留 fun 映射兜底
// （已软删的 fun 不会被 categories::list 拉出，map 留着无副作用）
const DEFAULT_CATEGORY_NAMES: Record<string, string> = {
  code: "编程",
  browse: "浏览",
  talk: "社交",
  design: "设计",
  fun: "娱乐",
  other: "其他",
  hidden: "隐藏",
  office: "办公",
  workchat: "工作沟通",
  game: "游戏",
  video: "影音",
};

// v29 / v31 / v32 / v33 migration 写入的默认大类映射，同款"未改名才翻译"逻辑
const DEFAULT_SUPER_CATEGORY_NAMES: Record<string, string> = {
  work: "工作",
  play: "娱乐",
  social: "社交",
  browse: "浏览",
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

/** 渲染大类名：默认大类（且未被改名）走 i18n，其余直接用 sup.name */
export function displaySuperCategoryName(
  sup: Pick<SuperCategory, "id" | "name">,
  t: TFunction,
): string {
  const original = DEFAULT_SUPER_CATEGORY_NAMES[sup.id];
  if (original !== undefined && sup.name === original) {
    return t(`categories.super.defaults.${sup.id}`);
  }
  return sup.name;
}
