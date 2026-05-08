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
import {
  DEFAULT_IMAGE_DESCRIBE_PROMPTS,
  DEFAULT_SYSTEM_PROMPTS,
} from "../../lib/aiPrompts";
import { useSettings } from "../../state/settings";

interface DebugState {
  debugMaxImages: number;
  setDebugMaxImages: (v: number) => void;
  debugExcluded: string[];
  setDebugExcluded: (v: string[]) => void;
  debugHashThreshold: number;
  setDebugHashThreshold: (v: number) => void;
  debugHashWindow: number;
  setDebugHashWindow: (v: number) => void;
  /** llama-server `--batch-size` / `--ubatch-size`；null = 走 llama.cpp 默认 */
  debugBatchSize: number | null;
  setDebugBatchSize: (v: number | null) => void;
  /** llama-server `-np` 并行槽位 */
  debugParallelSlots: number;
  setDebugParallelSlots: (v: number) => void;
  /** 每槽 ctx 上限；null = 8K 默认 */
  debugCtxSize: number | null;
  setDebugCtxSize: (v: number | null) => void;
  /** step 2 段总结 system prompt 文本 */
  debugSysPrompt: string;
  setDebugSysPrompt: (v: string) => void;
  /** step 1 单图描述 system prompt 文本 */
  debugImagePrompt: string;
  setDebugImagePrompt: (v: string) => void;
  /** 段总结走云端 (true) 还是本地 (false)；endpoint/model/apiKey 永远沿用全局 */
  debugExternalEnabled: boolean;
  setDebugExternalEnabled: (v: boolean) => void;
}

const DebugStateContext = createContext<DebugState | null>(null);

export function DebugStateProvider({ children }: { children: ReactNode }) {
  const { settings } = useSettings();

  const [debugMaxImages, setDebugMaxImages] = useState(30);
  const [debugExcluded, setDebugExcluded] = useState<string[]>([]);
  const [debugHashThreshold, setDebugHashThreshold] = useState(5);
  const [debugHashWindow, setDebugHashWindow] = useState(5);
  const [debugBatchSize, setDebugBatchSize] = useState<number | null>(null);
  const [debugParallelSlots, setDebugParallelSlots] = useState(1);
  const [debugCtxSize, setDebugCtxSize] = useState<number | null>(null);
  const [debugSysPrompt, setDebugSysPrompt] = useState("");
  const [debugImagePrompt, setDebugImagePrompt] = useState("");
  const [debugExternalEnabled, setDebugExternalEnabled] = useState(false);

  // settings 一加载就把 ai 字段拷成 debug 初值；只跑一次，之后用户在 debug tab
  // 改值都是本地的，不会被 settings 重新覆盖
  const initedRef = useRef(false);
  useEffect(() => {
    if (initedRef.current || !settings) return;
    initedRef.current = true;
    // maxImages snap 到 {15, 30, 100000}——picker 显示和 state 对齐，避免
    // 「settings 100、picker 显示 30、下发 100」的视觉错配
    const m = settings.ai.maxImagesPerSegment;
    setDebugMaxImages(m >= 1000 ? 100_000 : m >= 30 ? 30 : 15);
    setDebugExcluded(settings.ai.excludedCategories);
    setDebugHashThreshold(settings.ai.hashThreshold);
    setDebugHashWindow(settings.ai.hashWindowMinutes);
    setDebugBatchSize(settings.ai.batchSize ?? null);
    setDebugParallelSlots(settings.ai.parallelSlots ?? 1);
    setDebugCtxSize(settings.ai.ctxSize ?? null);
    // prompt：settings 覆盖优先，否则内置默认；保证 textarea 一打开就有真实文本
    const lang = settings.ai.promptLanguage;
    const sysOverride = settings.ai.promptOverrides[
      lang === "en" ? "systemEn" : lang === "ja" ? "systemJa" : "systemZh"
    ];
    const imgOverride = settings.ai.imageDescribeOverrides?.[
      lang === "en" ? "systemEn" : lang === "ja" ? "systemJa" : "systemZh"
    ] ?? "";
    setDebugSysPrompt(sysOverride.trim() || DEFAULT_SYSTEM_PROMPTS[lang]);
    setDebugImagePrompt(imgOverride.trim() || DEFAULT_IMAGE_DESCRIBE_PROMPTS[lang]);
    setDebugExternalEnabled(settings.ai.externalEnabled ?? false);
  }, [settings]);

  const value: DebugState = {
    debugMaxImages,
    setDebugMaxImages,
    debugExcluded,
    setDebugExcluded,
    debugHashThreshold,
    setDebugHashThreshold,
    debugHashWindow,
    setDebugHashWindow,
    debugBatchSize,
    setDebugBatchSize,
    debugParallelSlots,
    setDebugParallelSlots,
    debugCtxSize,
    setDebugCtxSize,
    debugSysPrompt,
    setDebugSysPrompt,
    debugImagePrompt,
    setDebugImagePrompt,
    debugExternalEnabled,
    setDebugExternalEnabled,
  };

  return (
    <DebugStateContext.Provider value={value}>
      {children}
    </DebugStateContext.Provider>
  );
}

export function useDebugState(): DebugState {
  const ctx = useContext(DebugStateContext);
  if (!ctx) {
    throw new Error("useDebugState must be used within <DebugStateProvider>");
  }
  return ctx;
}
