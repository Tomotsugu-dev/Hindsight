// AI 总结页面跨 tab 共享的「调试参数」state。
//
// DebugTab（跑总结、看结果）和 DebugSettingsTab（参数配置）是两个独立路由元素，
// 各自 mount/unmount 时 useState 不共享。把这些参数 state 提到 AISummaryPage
// 上层 Context 里，两个 tab 都能 useDebugState() 拿到同一份。
//
// settings.ai 是用户全局值；这里 init 一次性从 settings 拷贝默认值，之后两个
// debug tab 内改值都是 local，不回写 settings——跟原 DebugTab 行为一致。

import {
  createContext,
  useContext,
  useEffect,
  useRef,
  useState,
  type ReactNode,
} from "react";
import { DEFAULT_SYSTEM_PROMPTS, overrideKey } from "../../lib/aiPrompts";
import { useSettings } from "../../state/settings";

interface DebugState {
  debugExcluded: string[];
  setDebugExcluded: (v: string[]) => void;
  /** 段总结阶段 batch；null = fallback 到默认 */
  debugSummaryBatchSize: number | null;
  setDebugSummaryBatchSize: (v: number | null) => void;
  /** 段总结阶段 -np（推荐恒为 1） */
  debugSummaryParallelSlots: number;
  setDebugSummaryParallelSlots: (v: number) => void;
  /** 段总结阶段每槽 ctx；null = 8K 默认 */
  debugSummaryCtxSize: number | null;
  setDebugSummaryCtxSize: (v: number | null) => void;
  /** 段总结 system prompt 文本 */
  debugSysPrompt: string;
  setDebugSysPrompt: (v: string) => void;
  /** 段总结走云端 (true) 还是本地 (false)；endpoint/model/apiKey 永远沿用全局 */
  debugExternalEnabled: boolean;
  setDebugExternalEnabled: (v: boolean) => void;
}

const DebugStateContext = createContext<DebugState | null>(null);

export function DebugStateProvider({ children }: { children: ReactNode }) {
  const { settings } = useSettings();

  const [debugExcluded, setDebugExcluded] = useState<string[]>([]);
  const [debugSummaryBatchSize, setDebugSummaryBatchSize] = useState<number | null>(null);
  const [debugSummaryParallelSlots, setDebugSummaryParallelSlots] = useState(1);
  const [debugSummaryCtxSize, setDebugSummaryCtxSize] = useState<number | null>(null);
  const [debugSysPrompt, setDebugSysPrompt] = useState("");
  const [debugExternalEnabled, setDebugExternalEnabled] = useState(false);

  // settings 一加载就把 ai 字段拷成 debug 初值；只跑一次，之后用户在 debug tab
  // 改值都是本地的，不会被 settings 重新覆盖
  const initedRef = useRef(false);
  useEffect(() => {
    if (initedRef.current || !settings) return;
    initedRef.current = true;
    setDebugExcluded(settings.ai.excludedCategories);
    // 调试参数初值——优先用 summary 专用字段，未设则 fallback 到旧全局字段
    setDebugSummaryBatchSize(
      settings.ai.summaryBatchSize ?? settings.ai.batchSize ?? null,
    );
    setDebugSummaryParallelSlots(
      settings.ai.summaryParallelSlots ?? settings.ai.parallelSlots ?? 1,
    );
    setDebugSummaryCtxSize(
      settings.ai.summaryCtxSize ?? settings.ai.ctxSize ?? null,
    );
    // prompt：settings 覆盖优先，否则内置默认；保证 textarea 一打开就有真实文本
    const lang = settings.ai.promptLanguage;
    const key = overrideKey(lang);
    const sysOverride = settings.ai.promptOverrides[key];
    setDebugSysPrompt(sysOverride.trim() || DEFAULT_SYSTEM_PROMPTS[lang]);
    setDebugExternalEnabled(settings.ai.externalEnabled ?? false);
  }, [settings]);

  const value: DebugState = {
    debugExcluded,
    setDebugExcluded,
    debugSummaryBatchSize,
    setDebugSummaryBatchSize,
    debugSummaryParallelSlots,
    setDebugSummaryParallelSlots,
    debugSummaryCtxSize,
    setDebugSummaryCtxSize,
    debugSysPrompt,
    setDebugSysPrompt,
    debugExternalEnabled,
    setDebugExternalEnabled,
  };

  return (
    <DebugStateContext.Provider value={value}>
      {children}
    </DebugStateContext.Provider>
  );
}

// Provider + 配套 hook 同文件是 React Context 的标准布局；为消除 Vite Fast Refresh
// 对"组件 + 非组件"混合导出的告警，单独抑制这一行。
// eslint-disable-next-line react-refresh/only-export-components
export function useDebugState(): DebugState {
  const ctx = useContext(DebugStateContext);
  if (!ctx) {
    throw new Error("useDebugState must be used within <DebugStateProvider>");
  }
  return ctx;
}
