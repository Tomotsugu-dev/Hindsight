import { useCallback } from "react";
import { useTranslation } from "react-i18next";

/**
 * 把分钟数格式化成本地化的"X 小时 Y 分钟"文案。
 * 走 i18n（common.duration.*）所以必须在 React 组件树里用。
 *
 * 抽自 Today/Week/Month 三页内的 fmtHM 函数（三处一字不差）。
 */
export function useDurationFormatter(): (min: number) => string {
  const { t } = useTranslation();
  return useCallback(
    (min: number) => {
      const h = Math.floor(min / 60);
      const m = min % 60;
      if (h === 0) return t("common.duration.minutesShort", { count: m });
      return t("common.duration.hourMinute", { hours: h, minutes: m });
    },
    [t],
  );
}
