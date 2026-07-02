import { useCallback } from "react";
import { useTranslation } from "react-i18next";

/**
 * 图表轴刻度专用的英文短格式：45m / 1h / 1.25h / 16.5h。
 * 轴刻度全语言统一用英文（紧凑、免本地化宽度问题）；句子语境（tooltip / 页头 /
 * 列表）仍走 useDurationFormatter 的本地化文案。抽自 HourlyChart 的 formatMinLabel。
 */
export function formatAxisTick(min: number): string {
  if (min === 0) return "0";
  if (min < 60) return `${min}m`;
  if (min % 60 === 0) return `${min / 60}h`;
  // 0.25h 档位（1.25 / 1.5 / 1.75 / ...）：保留 2 位小数后用 parseFloat 去尾 0
  // 避免 toFixed(1) 把 1.25 round 成 "1.3h"
  return `${parseFloat((min / 60).toFixed(2))}h`;
}

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
