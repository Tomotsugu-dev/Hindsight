import type { TFunction } from "i18next";
import { SUMMARY_CLOUD_SENTINEL, type AiConfig } from "../../api/hindsight";
import {
  FOLLOW_DAILY,
  reportRoute,
  type ReportModelState,
  type ReportModule,
  type ReportRoute,
} from "../../state/reportModel";

export interface ModelOption {
  value: string;
  label: string;
  kind: "follow" | "cloud" | "local";
  /** 右边的灰字：「跟日报一样」后面写日报现在用的模型 */
  detail?: string;
}

/** 文件名去掉 .gguf，好放进窄的地方 */
export function shortModelName(file: string): string {
  return file.replace(/\.gguf$/i, "");
}

/** 能选的模型：周报先是「跟日报一样」，然后是填好的云端，最后是下载了的本地模型。 */
export function modelOptions(
  m: ReportModelState,
  module: ReportModule,
  t: TFunction,
): ModelOption[] {
  const options: ModelOption[] = [];
  if (module === "weekly") {
    options.push({
      value: FOLLOW_DAILY,
      label: t("aiSummary.model.followDaily"),
      kind: "follow",
      detail: routeName(reportRoute(m.ai, "daily"), m.ai, t),
    });
  }
  if (m.cloudReady) {
    options.push({
      value: SUMMARY_CLOUD_SENTINEL,
      label: t("chat.badge.cloud", { model: m.ai.model.trim() }),
      kind: "cloud",
    });
  }
  // 在用的文件不在下载列表里（比如还没列出来）时也列上，不然下拉框上一个字都没有
  const inUse = checkedModel(m, module);
  const files =
    inUse && inUse === m.route.local && !m.localFiles.includes(inUse)
      ? [inUse, ...m.localFiles]
      : m.localFiles;
  for (const file of files) {
    options.push({ value: file, label: shortModelName(file), kind: "local" });
  }
  return options;
}

/** 打勾的那一项；一项都对不上（比如还没选过模型）时是 null。 */
export function checkedModel(m: ReportModelState, module: ReportModule): string | null {
  if (module === "weekly" && m.value === FOLLOW_DAILY) return FOLLOW_DAILY;
  if (m.route.cloud) return SUMMARY_CLOUD_SENTINEL;
  return m.route.local === "" ? null : m.route.local;
}

/** 实际在用的模型，带「本地」「云端」；周报跟日报时就是日报的模型。 */
export function routeLabel(m: ReportModelState, t: TFunction): string {
  if (m.route.cloud) return t("chat.badge.cloud", { model: m.ai.model.trim() });
  if (m.route.local) return t("chat.badge.local", { model: shortModelName(m.route.local) });
  return t("chat.badge.none");
}

/** 同上，只有模型名，给放不下的地方用（本地、云端靠图标分） */
export function routeName(route: ReportRoute, ai: AiConfig, t: TFunction): string {
  if (route.cloud) return ai.model.trim();
  if (route.local) return shortModelName(route.local);
  return t("chat.badge.none");
}
