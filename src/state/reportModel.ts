import { useCallback, useEffect, useState } from "react";
import {
  api,
  type AdvancedEntry,
  type AiConfig,
  type CloudConnection,
  type ModelPick,
  type ModelRef,
  type ReasoningLevel,
} from "../api/hindsight";
import { logError } from "../lib/logger";
import { normalizeThinking, thinkingModes } from "../lib/thinkingModes";
import {
  findCloudConnection,
  isCloudConnectionReady,
  readyCloudConnections,
} from "../pages/Chat/chatRouting";
import { useSettings } from "./settings";

export type ReportModule = "daily" | "weekly";
export type AiTask = "daily" | "weekly" | "chat";

/** 周报的模型选「跟日报一样」时，下拉框里这一项的值 */
export const FOLLOW_DAILY = "";
export const CLOUD_VALUE_PREFIX = "cloud:";
export const LOCAL_VALUE_PREFIX = "local:";

export function encodeModelRef(model: ModelRef): string {
  return model.kind === "cloud"
    ? `${CLOUD_VALUE_PREFIX}${model.connection}`
    : `${LOCAL_VALUE_PREFIX}${model.file}`;
}

export function decodeModelValue(value: string): ModelPick | null {
  if (value === FOLLOW_DAILY) return { kind: "inherit" };
  if (value.startsWith(CLOUD_VALUE_PREFIX)) {
    const connection = value.slice(CLOUD_VALUE_PREFIX.length).trim();
    return connection ? { kind: "cloud", connection } : null;
  }
  if (value.startsWith(LOCAL_VALUE_PREFIX)) {
    const file = value.slice(LOCAL_VALUE_PREFIX.length).trim();
    return file ? { kind: "local", file } : null;
  }
  const file = value.trim();
  return file ? { kind: "local", file } : null;
}

export function sameModel(a: ModelRef, b: ModelRef): boolean {
  if (a.kind !== b.kind) return false;
  return a.kind === "cloud"
    ? a.connection === (b as { kind: "cloud"; connection: string }).connection
    : a.file === (b as { kind: "local"; file: string }).file;
}

export function findAdvanced(
  ai: AiConfig,
  task: AiTask,
  model: ModelRef,
): AdvancedEntry | undefined {
  return ai.advanced.find((e) => e.task === task && sameModel(e.model, model));
}

export function patchAdvanced(
  ai: AiConfig,
  task: AiTask,
  model: ModelRef,
  patch: Partial<Pick<AdvancedEntry, "reasoning" | "ctxSize" | "parallelSlots" | "batchSize">>,
): AdvancedEntry[] {
  const prev = findAdvanced(ai, task, model);
  const next: AdvancedEntry = {
    task,
    model,
    reasoning: patch.reasoning !== undefined ? patch.reasoning : (prev?.reasoning ?? null),
    ctxSize: patch.ctxSize !== undefined ? patch.ctxSize : (prev?.ctxSize ?? null),
    parallelSlots:
      patch.parallelSlots !== undefined ? patch.parallelSlots : (prev?.parallelSlots ?? null),
    batchSize: patch.batchSize !== undefined ? patch.batchSize : (prev?.batchSize ?? null),
  };
  const rest = ai.advanced.filter((e) => !(e.task === task && sameModel(e.model, model)));
  const empty =
    next.reasoning === null &&
    next.ctxSize === null &&
    next.parallelSlots === null &&
    next.batchSize === null;
  return empty ? rest : [...rest, next];
}

/** 下拉框里的值：周报可为 FOLLOW_DAILY，其余为 `cloud:<id>` 或 `local:<file>`，未选为空串。 */
export function reportModelValue(ai: AiConfig, module: ReportModule): string {
  if (module === "daily") {
    return ai.picks.daily ? encodeModelRef(ai.picks.daily) : "";
  }
  const pick = ai.picks.weekly;
  if (pick.kind === "inherit") return FOLLOW_DAILY;
  return encodeModelRef(pick);
}

export interface ReportRoute {
  /** 走云端时的可用连接；否则 null */
  cloud: CloudConnection | null;
  /** 走本地时的文件名；否则空串 */
  local: string;
}

function resolveModelRef(ai: AiConfig, model: ModelRef | null): ReportRoute {
  if (!model) return { cloud: null, local: "" };
  if (model.kind === "local") {
    return { cloud: null, local: model.file.trim() };
  }
  const conn = findCloudConnection(ai, model.connection);
  return conn && isCloudConnectionReady(conn)
    ? { cloud: conn, local: "" }
    : { cloud: null, local: "" };
}

/** 报告实际用哪个模型，镜像后端 `AiConfig::model_for`。 */
export function reportRoute(ai: AiConfig, module: ReportModule): ReportRoute {
  if (module === "daily") {
    return resolveModelRef(ai, ai.picks.daily);
  }
  const pick = ai.picks.weekly;
  if (pick.kind === "inherit") return reportRoute(ai, "daily");
  return resolveModelRef(ai, pick);
}

/** 实际用的模型引用，用来在 `advanced` 里定位参数覆盖。 */
export function routeModel(route: ReportRoute): ModelRef | null {
  if (route.cloud) return { kind: "cloud", connection: route.cloud.id };
  if (route.local) return { kind: "local", file: route.local };
  return null;
}

/** 存着的思考档位：`advanced` 里这份报告 + 实际用的模型那一项；没有就是 "auto"。 */
export function reportThinking(ai: AiConfig, module: ReportModule): string {
  const model = routeModel(reportRoute(ai, module));
  if (!model) return "auto";
  return findAdvanced(ai, module, model)?.reasoning ?? "auto";
}

/** 改了思考档位以后的 `advanced`；选 "auto" 清掉 reasoning 字段。 */
export function withReportThinking(
  ai: AiConfig,
  module: ReportModule,
  mode: string,
): AdvancedEntry[] {
  const model = routeModel(reportRoute(ai, module));
  if (!model) return ai.advanced;
  return patchAdvanced(ai, module, model, {
    reasoning: mode === "auto" ? null : (mode as ReasoningLevel),
  });
}

/** 报告能选的思考档位，跟着实际用的服务商方言走。报告不发工具。 */
export function reportThinkingModes(ai: AiConfig, module: ReportModule): string[] {
  return thinkingModes(reportRoute(ai, module).cloud?.dialect ?? null, false);
}

/**
 * 一份报告（日报或周报）的模型和思考模式：现在用的是什么、能选什么、怎么改。
 * 选择存进设置，自动生成也用它。改成云端前先要用户确认，因为活动记录会发出去。
 */
export function useReportModel(module: ReportModule) {
  const { settings, update, reload } = useSettings();
  const [localFiles, setLocalFiles] = useState<string[]>([]);
  const [pendingCloudValue, setPendingCloudValue] = useState<string | null>(null);

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
  const cloudConnections = readyCloudConnections(ai);

  const pendingCloudConnection =
    pendingCloudValue && pendingCloudValue.startsWith(CLOUD_VALUE_PREFIX)
      ? findCloudConnection(ai, pendingCloudValue.slice(CLOUD_VALUE_PREFIX.length))
      : null;

  const save = async (value: string) => {
    const decoded = decodeModelValue(value);
    const pick: ModelPick | null =
      module === "weekly"
        ? (decoded ?? { kind: "inherit" })
        : decoded && decoded.kind !== "inherit"
          ? decoded
          : null;
    try {
      await api.setTaskModel(module, pick);
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
    /** 可用的云端连接列表 */
    cloudConnections,
    localFiles,
    refreshLocal,
    modes,
    thinking: normalizeThinking(reportThinking(ai, module), modes),
    /** 换模型。从本地/未选换到云端时先等用户在确认框里点同意 */
    chooseModel: (value: string) => {
      if (value.startsWith(CLOUD_VALUE_PREFIX) && !route.cloud) {
        setPendingCloudValue(value);
      } else {
        void save(value);
      }
    },
    askingCloud: pendingCloudValue !== null,
    pendingCloudConnection,
    confirmCloud: () => {
      const value = pendingCloudValue;
      setPendingCloudValue(null);
      if (value) void save(value);
    },
    cancelCloud: () => setPendingCloudValue(null),
    /** 换思考档位。存在「这份报告 + 现在用的模型」下面，换了模型就是那个模型自己的档位 */
    chooseThinking: (mode: string) =>
      update({ ai: { ...ai, advanced: withReportThinking(ai, module, mode) } }),
  };
}

export type ReportModelState = NonNullable<ReturnType<typeof useReportModel>>;
