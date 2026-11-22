import type { TFunction } from "i18next";
import {
  CLOUD_VALUE_PREFIX,
  FOLLOW_DAILY,
  LOCAL_VALUE_PREFIX,
  reportRoute,
  type ReportModelState,
  type ReportModule,
  type ReportRoute,
} from "../../state/reportModel";
import { profileLabel } from "../../utils/profileLabel";

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

/** 能选的模型：周报先是「跟日报一样」，然后是可用的云端连接，最后是下载了的本地模型。 */
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
      detail: routeName(reportRoute(m.ai, "daily"), t),
    });
  }
  for (const conn of m.cloudConnections) {
    options.push({
      value: `${CLOUD_VALUE_PREFIX}${conn.id}`,
      label: t("chat.badge.cloud", { model: profileLabel(conn) }),
      kind: "cloud",
    });
  }
  // 在用的文件不在下载列表里（比如还没列出来）时也列上，不然下拉框上一个字都没有
  const localInUse = m.route.local;
  const files =
    localInUse && !m.localFiles.includes(localInUse) ? [localInUse, ...m.localFiles] : m.localFiles;
  for (const file of files) {
    options.push({
      value: `${LOCAL_VALUE_PREFIX}${file}`,
      label: shortModelName(file),
      kind: "local",
    });
  }
  return options;
}

/** 打勾的那一项；一项都对不上（比如还没选过模型）时是 null。 */
export function checkedModel(m: ReportModelState, module: ReportModule): string | null {
  if (module === "weekly" && m.value === FOLLOW_DAILY) return FOLLOW_DAILY;
  if (m.route.cloud) return `${CLOUD_VALUE_PREFIX}${m.route.cloud.id}`;
  return m.route.local === "" ? null : `${LOCAL_VALUE_PREFIX}${m.route.local}`;
}

/** 实际在用的模型，带「本地」「云端」；周报跟日报时就是日报的模型。 */
export function routeLabel(m: ReportModelState, t: TFunction): string {
  if (m.route.cloud) return t("chat.badge.cloud", { model: profileLabel(m.route.cloud) });
  if (m.route.local) return t("chat.badge.local", { model: shortModelName(m.route.local) });
  return t("chat.badge.none");
}

/** 同上，只有模型名，给放不下的地方用（本地、云端靠图标分） */
export function routeName(route: ReportRoute, t: TFunction): string {
  if (route.cloud) return route.cloud.model.trim() || profileLabel(route.cloud);
  if (route.local) return shortModelName(route.local);
  return t("chat.badge.none");
}
