import i18n from "../i18n";
import { api } from "../api/hindsight";

/**
 * 把还是中文原名的默认分类、默认大类换成当前界面语言的译名（ADR-0015）。
 * 每次启动、渲染前调一次：后端只换还没换过的，之后再调什么都不改。失败不挡启动。
 */
export async function translateDefaultCategoryNames(): Promise<void> {
  const categories = i18n.t("categories.defaults", { returnObjects: true }) as unknown as Record<
    string,
    string
  >;
  const superCategories = i18n.t("categories.super.defaults", {
    returnObjects: true,
  }) as unknown as Record<string, string>;
  try {
    await api.translateDefaultCategoryNames(categories, superCategories);
  } catch (e) {
    console.warn("translateDefaultCategoryNames failed", e);
  }
}
