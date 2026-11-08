import type { TFunction } from "i18next";
import type { DataCount } from "../api/hindsight";
import { formatBytes } from "./formatBytes";

/** 内置「隐私」分类的 id：归到这里的应用和网站不记录。 */
export const PRIVATE_CATEGORY_ID = "private";

/** 内置「不计入统计」分类的 id。界面上的名字以前叫「隐藏」，id 没改。 */
export const EXCLUDED_CATEGORY_ID = "hidden";

/** 设置存好了、删除失败时，后端错误的前缀。同后端 `privacy::ERR_PREFIX_PURGE_FAILED`。 */
const PURGE_FAILED_PREFIX = "[PURGE_FAILED] ";

/** 错误的文字。Tauri 命令的错误是字符串。 */
export function errorText(e: unknown): string {
  return e instanceof Error ? e.message : String(e);
}

/** 设置存好了、删除失败时，返回删除失败的原因；别的错误返回 null。 */
export function purgeFailedReason(e: unknown): string | null {
  const text = errorText(e);
  return text.startsWith(PURGE_FAILED_PREFIX) ? text.slice(PURGE_FAILED_PREFIX.length) : null;
}

/** 「已有的 1,234 条记录和 56 张截图（210.0 MB）」。还没统计出来时是 null，说「已有的全部记录和截图」。 */
export function describeCount(t: TFunction, count: DataCount | null): string {
  if (count === null) return t("categories.private.count.unknown");
  const records = t("categories.private.count.records", { count: count.activities });
  const data =
    count.screenshots === 0
      ? records
      : t("categories.private.count.both", {
          records,
          screenshots: t("categories.private.count.screenshots", {
            count: count.screenshots,
            size: formatBytes(count.screenshotBytes),
          }),
        });
  return t("categories.private.count.known", { data });
}
