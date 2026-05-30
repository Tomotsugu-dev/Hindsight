import type { TFunction } from "i18next";
import type { SummaryProgress } from "../../../api/hindsight";

/** 事件流 log 单条。 */
export interface LogEntry {
  ts: string; // HH:MM:SS.mmm
  phase: SummaryProgress["phase"];
  body: string;
}

/** 调试 tab 顶部的"调什么"——目前只有日报有真后端，周报 / 月报先占位。 */
export type DebugScope = "daily" | "weekly" | "monthly";

export function buildScopeOptions(
  t: TFunction,
): Array<{ value: DebugScope; label: string }> {
  return [
    { value: "daily", label: t("aiSummary.debug.scope.daily") },
    { value: "weekly", label: t("aiSummary.debug.scope.weekly") },
    { value: "monthly", label: t("aiSummary.debug.scope.monthly") },
  ];
}

export const LOG_RING_SIZE = 200; // 防止整日跑事件流爆内存

function fmtLocalDate(d: Date): string {
  const y = d.getFullYear();
  const m = String(d.getMonth() + 1).padStart(2, "0");
  const day = String(d.getDate()).padStart(2, "0");
  return `${y}-${m}-${day}`;
}

/** 按 scope 把 offset 解释成具体的"锚定日期"。
 *
 * - daily   →  当天本身（offset = 距今多少天）
 * - weekly  →  该周的周一（offset = 距本周多少周；以 周一为周起点）
 * - monthly →  该月 1 号（offset = 距本月多少月）
 *
 * 后端实现周报 / 月报命令时，约定传这个 "周一日期 / 月初日期" 作为 anchor。 */
export function anchorDateStr(scope: DebugScope, offset: number): string {
  const d = new Date();
  if (scope === "daily") {
    d.setDate(d.getDate() + offset);
  } else if (scope === "weekly") {
    // JS 的 getDay() 周日=0，调整为周一=0
    const dow = (d.getDay() + 6) % 7;
    d.setDate(d.getDate() - dow + offset * 7);
  } else {
    d.setDate(1);
    d.setMonth(d.getMonth() + offset);
  }
  return fmtLocalDate(d);
}

/** 按 scope + offset 给 dayPill 显示的文案。 */
export function offsetLabel(
  scope: DebugScope,
  offset: number,
  t: TFunction,
): string {
  if (scope === "daily") {
    if (offset === 0) return t("aiSummary.debug.dateNav.today");
    if (offset === -1) return t("aiSummary.debug.dateNav.yesterday");
    return anchorDateStr("daily", offset);
  }
  if (scope === "weekly") {
    if (offset === 0) return t("aiSummary.debug.dateNav.thisWeek");
    if (offset === -1) return t("aiSummary.debug.dateNav.lastWeek");
    if (offset === -2) return t("aiSummary.debug.dateNav.weekBeforeLast");
    return t("aiSummary.debug.dateNav.weeksAgo", { count: -offset });
  }
  // monthly
  if (offset === 0) return t("aiSummary.debug.dateNav.thisMonth");
  if (offset === -1) return t("aiSummary.debug.dateNav.lastMonth");
  return t("aiSummary.debug.dateNav.monthsAgo", { count: -offset });
}

/** platform_id 是 binary 变体路由 ID（"win-cuda-13.1-x64" 等），不是 OS 平台。
 *  转成人话标签给状态条显示。跟 [AISettings.tsx::humanAccelLabel] 同步维护。 */
export function humanAccelLabel(platformId: string): string {
  switch (platformId) {
    case "win-cuda-12.4-x64":
      return "CUDA 12.4";
    case "win-cuda-13.1-x64":
      return "CUDA 13.1";
    case "win-cpu-x64":
      return "CPU";
    case "macos-arm64":
      return "Apple Silicon · Metal";
    case "macos-x64":
      return "Intel Mac";
    case "ubuntu-x64":
      return "Linux CPU";
    default:
      return platformId;
  }
}

export function nowHms(): string {
  const d = new Date();
  const hh = String(d.getHours()).padStart(2, "0");
  const mm = String(d.getMinutes()).padStart(2, "0");
  const ss = String(d.getSeconds()).padStart(2, "0");
  const ms = String(d.getMilliseconds()).padStart(3, "0");
  return `${hh}:${mm}:${ss}.${ms}`;
}

/** 把 phase + payload 浓缩成一行 log body 字符串。 */
export function fmtPhaseBody(p: SummaryProgress): string {
  const parts: string[] = [];
  if (p.segmentIdx != null) parts.push(`idx=${p.segmentIdx}`);
  if (p.imageIndex != null) parts.push(`img=${p.imageIndex}`);
  if (p.imagesTotal != null) parts.push(`total=${p.imagesTotal}`);
  if (p.status != null) parts.push(`status=${p.status}`);
  if (p.message) parts.push(p.message);
  if (p.imageDescription) {
    const short = p.imageDescription.replace(/\s+/g, " ").slice(0, 80);
    parts.push(`"${short}${p.imageDescription.length > 80 ? "…" : ""}"`);
  }
  return parts.join(" · ");
}
