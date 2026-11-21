import { useCallback, useEffect, useState } from "react";
import {
  api,
  SUMMARY_CLOUD_SENTINEL,
  type AdvancedEntry,
  type AiConfig,
  type ModelPick,
  type ModelRef,
  type ReasoningLevel,
} from "../api/hindsight";
import { logError } from "../lib/logger";
import { normalizeThinking, thinkingModes } from "../lib/thinkingModes";
import { chatCloudReady } from "../pages/Chat/chatRouting";
import { useSettings } from "./settings";

export type ReportModule = "daily" | "weekly";

/** 周报的模型选「跟日报一样」时，下拉框里这一项的值 */
export const FOLLOW_DAILY = "";

/**
 * 下拉框里的值。日报是 summaryMain；周报把 picks.weekly 换成同一种写法：
 * 跟日报一样是 FOLLOW_DAILY，云端是 SUMMARY_CLOUD_SENTINEL，本地是文件名。
 */
export function reportModelValue(ai: AiConfig, module: ReportModule): string {
  if (module === "daily") return ai.summaryMain.trim();
  const pick = ai.picks?.weekly;
  if (pick?.kind === "local") return pick.file.trim();
  if (pick?.kind === "cloud") return SUMMARY_CLOUD_SENTINEL;
  return FOLLOW_DAILY;
}

/** 下拉框里的值换回 picks.weekly */
function weeklyPick(value: string): ModelPick {
  if (value === FOLLOW_DAILY) return { kind: "inherit" };
  if (value === SUMMARY_CLOUD_SENTINEL) return { kind: "cloud" };
  return { kind: "local", file: value };
}

export interface ReportRoute {
  /** 走云端 */
  cloud: boolean;
  /** 不走云端时用的本地文件；空 = 没有可用的本地模型 */
  local: string;
}

/**
 * 报告实际用哪个模型，照后端 AiConfig::task_model 算：
 * 选了云端、云端也开着 → 云端；选了本地文件 → 这个文件；其余 → activeMain。
 * 周报跟日报一样时，就是日报的结果。
 */
export function reportRoute(ai: AiConfig, module: ReportModule): ReportRoute {
  const value = reportModelValue(ai, module);
  if (module === "weekly" && value === FOLLOW_DAILY) return reportRoute(ai, "daily");
  const cloud = value === SUMMARY_CLOUD_SENTINEL && ai.externalEnabled;
  const local = value !== "" && value !== SUMMARY_CLOUD_SENTINEL ? value : ai.activeMain.trim();
  return { cloud, local };
}

/** 实际用的模型，写法同后端的 ModelRef，用来在 advanced 里找这一项 */
function routeModel(route: ReportRoute): ModelRef {
  return route.cloud ? { kind: "cloud" } : { kind: "local", file: route.local };
}

function sameModel(a: ModelRef, b: ModelRef): boolean {
  if (a.kind === "cloud" || b.kind === "cloud") return a.kind === b.kind;
  return a.file === b.file;
}

/** 这份报告、实际用的这个模型在 advanced 里的那一项 */
function isEntryOf(entry: AdvancedEntry, module: ReportModule, model: ModelRef): boolean {
  return entry.task === module && sameModel(entry.model, model);
}

/** 存着的思考档位：advanced 里这份报告、实际用的这个模型的那一项；没有就是 "auto"。 */
export function reportThinking(ai: AiConfig, module: ReportModule): string {
  const model = routeModel(reportRoute(ai, module));
  return (ai.advanced ?? []).find((e) => isEntryOf(e, module, model))?.reasoning ?? "auto";
}

/** 改了思考档位以后的 advanced：换掉这一项；选 "auto" 就删掉它。 */
export function withReportThinking(
  ai: AiConfig,
  module: ReportModule,
  mode: string,
): AdvancedEntry[] {
  const model = routeModel(reportRoute(ai, module));
  const rest = (ai.advanced ?? []).filter((e) => !isEntryOf(e, module, model));
  if (mode === "auto") return rest;
  return [...rest, { task: module, model, reasoning: mode as ReasoningLevel }];
}

/** 报告能选的思考档位，跟着实际用的服务商走。报告不发工具。 */
export function reportThinkingModes(ai: AiConfig, module: ReportModule): string[] {
  return thinkingModes(reportRoute(ai, module).cloud ? ai.externalProvider : null, false);
}

/**
 * 一份报告（日报或周报）的模型和思考模式：现在用的是什么、能选什么、怎么改。
 * 选择存进设置，自动生成也用它。改成云端前先要用户确认，因为活动记录会发出去。
 */
export function useReportModel(module: ReportModule) {
  const { settings, update, reload } = useSettings();
  const [localFiles, setLocalFiles] = useState<string[]>([]);
  const [askingCloud, setAskingCloud] = useState(false);

  const refreshLocal = useCallback(() => {
    api
      .listLocalModels()
      .then((all) => setLocalFiles(all.filter((m) => !m.isMmproj).map((m) => m.filename)))
      .catch((e) => logError("reportModel.listModels", e));
  }, []);

  useEffect(refreshLocal, [refreshLocal]);

  if (!settings) return null;
  const ai = settings.ai;
  const route = reportRoute(ai, module);
  const modes = reportThinkingModes(ai, module);

  const save = async (value: string) => {
    if (module === "weekly") {
      update({ ai: { ...ai, picks: { ...ai.picks, weekly: weeklyPick(value) } } });
      return;
    }
    // 日报的模型有自己的命令，它会顺带停掉用不上的本地引擎。报告只用文字，不带 mmproj
    try {
      await api.setStepModel("summary", value, null);
      await reload();
    } catch (e) {
      logError("reportModel.setModel", e);
    }
  };

  return {
    ai,
    route,
    /** 存着的模型选择，周报可能是 FOLLOW_DAILY */
    value: reportModelValue(ai, module),
    /** 云端 API 填齐了，能选云端 */
    cloudReady: chatCloudReady(ai),
    localFiles,
    refreshLocal,
    modes,
    thinking: normalizeThinking(reportThinking(ai, module), modes),
    /** 换模型。换到云端时先等用户在确认框里点同意 */
    chooseModel: (value: string) => {
      if (value === SUMMARY_CLOUD_SENTINEL && !route.cloud) setAskingCloud(true);
      else void save(value);
    },
    askingCloud,
    confirmCloud: () => {
      setAskingCloud(false);
      void save(SUMMARY_CLOUD_SENTINEL);
    },
    cancelCloud: () => setAskingCloud(false),
    /** 换思考档位。存在「这份报告 + 现在用的模型」下面，换了模型就是那个模型自己的档位 */
    chooseThinking: (mode: string) =>
      update({ ai: { ...ai, advanced: withReportThinking(ai, module, mode) } }),
  };
}

export type ReportModelState = NonNullable<ReturnType<typeof useReportModel>>;
