import { useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import type { TFunction } from "i18next";
import { listen } from "@tauri-apps/api/event";
import { openPath, revealItemInDir } from "@tauri-apps/plugin-opener";
import { useMouseGlow } from "../../../hooks/useMouseGlow";
import {
  AlertTriangle,
  ChevronLeft,
  ChevronRight,
  Clock,
  Download,
  Filter,
  Image as ImageIcon,
  Info,
  Loader2,
  MessageSquareText,
  Newspaper,
  Play,
  RotateCcw,
  Server,
  Square,
  Trash2,
} from "lucide-react";
import {
  api,
  SUMMARY_PROGRESS_EVENT,
  type AiOverrides,
  type EngineStatus,
  type ImageDescriptionRow,
  type SegmentSummaryRow,
  type SummaryProgress,
} from "../../../api/hindsight";
import { useSettings } from "../../../state/settings";
import { SimplePicker } from "../../../components/SimplePicker/SimplePicker";
import { CategoryChipMultiSelect } from "../../Settings/components/CategoryChipMultiSelect";
import { Row } from "../../Settings/components/Row";
import { Section } from "../../Settings/components/Section";
import { Slider } from "../../Settings/components/Slider";
import {
  DEFAULT_IMAGE_DESCRIBE_PROMPTS,
  DEFAULT_SYSTEM_PROMPTS,
} from "../../../lib/aiPrompts";
import styles from "./DebugTab.module.css";

/** 事件流 log 单条。 */
interface LogEntry {
  ts: string; // HH:MM:SS.mmm
  phase: SummaryProgress["phase"];
  body: string;
}

/** 调试 tab 顶部的"调什么"——目前只有日报有真后端，周报 / 月报先占位。 */
type DebugScope = "daily" | "weekly" | "monthly";

function buildScopeOptions(t: TFunction): Array<{ value: DebugScope; label: string }> {
  return [
    { value: "daily", label: t("aiSummary.debug.scope.daily") },
    { value: "weekly", label: t("aiSummary.debug.scope.weekly") },
    { value: "monthly", label: t("aiSummary.debug.scope.monthly") },
  ];
}

/** 单段最大图片数选项。"max" 映射到 sanitize 上限 100000，"无限制"等于把段内
 *  所有截图全送给 LLM——撑爆 ctx 时 LLM 会返 400 被段标 error，用户回头再调小。 */
type MaxImagesKey = "15" | "30" | "max";
function buildMaxImagesOptions(t: TFunction): Array<{ value: MaxImagesKey; label: string }> {
  return [
    { value: "15", label: t("aiSummary.debug.pickerOptions.maxImages15") },
    { value: "30", label: t("aiSummary.debug.pickerOptions.maxImages30") },
    { value: "max", label: t("aiSummary.debug.pickerOptions.maxImagesUnlimited") },
  ];
}

function maxImagesToOption(n: number): MaxImagesKey {
  // 1000 起算"无限制"——这种大值正常路径不会出现，只有用户主动选 max 才会写入
  if (n >= 1000) return "max";
  if (n >= 30) return "30";
  return "15";
}
function optionToMaxImages(v: MaxImagesKey): number {
  if (v === "max") return 100_000;
  return parseInt(v, 10);
}

/** llama-server `--batch-size` / `--ubatch-size`。"default" = 不传，走 llama.cpp 默认 512。
 *  改值会触发引擎 stop+start 重启；调试跑完无条件 stop，下次正常日报跑回到默认。 */
type BatchKey = "default" | "1024" | "2048" | "4096";
// Batch 选项纯英文 + 数字，所有语言都保持一致，无需走 t()
const BATCH_OPTIONS: Array<{ value: BatchKey; label: string }> = [
  { value: "default", label: "Batch 512" },
  { value: "1024", label: "Batch 1024" },
  { value: "2048", label: "Batch 2048" },
  { value: "4096", label: "Batch 4096" },
];
function batchToOption(n: number | null): BatchKey {
  if (n === 1024) return "1024";
  if (n === 2048) return "2048";
  if (n === 4096) return "4096";
  return "default";
}
/** "default" → null（让 overrides.batchSize 留空，后端走默认）；其它 → 数值 */
function optionToBatch(v: BatchKey): number | null {
  return v === "default" ? null : parseInt(v, 10);
}

/** 并发槽位数 = llama-server `-np` + 后端 step 1 image describe 并发数。
 *  两边一致才有效。"1" = 串行（历史行为）；> 1 = 并发同时跑 N 张图描述。 */
type SlotsKey = "1" | "2" | "4" | "8";
function buildSlotsOptions(t: TFunction): Array<{ value: SlotsKey; label: string }> {
  return [
    { value: "1", label: t("aiSummary.debug.pickerOptions.slots1") },
    { value: "2", label: t("aiSummary.debug.pickerOptions.slots2") },
    { value: "4", label: t("aiSummary.debug.pickerOptions.slots4") },
    { value: "8", label: t("aiSummary.debug.pickerOptions.slots8") },
  ];
}
function slotsToOption(n: number): SlotsKey {
  if (n >= 8) return "8";
  if (n >= 4) return "4";
  if (n >= 2) return "2";
  return "1";
}
function optionToSlots(v: SlotsKey): number {
  return parseInt(v, 10);
}

/** 每 slot 的 ctx 上限（单位 token）。后端 `--ctx-size = ctxSize × slots`。
 *  启动时按总量一次性吃 KV cache（~30KB / token）；选大了 5090 也只是几 GB，
 *  CPU 用户保持「默认 (8K)」就行。 */
type CtxKey = "default" | "16384" | "32768" | "65536";
function buildCtxOptions(t: TFunction): Array<{ value: CtxKey; label: string }> {
  return [
    { value: "default", label: t("aiSummary.debug.pickerOptions.ctxDefault") },
    { value: "16384", label: t("aiSummary.debug.pickerOptions.ctx16k") },
    { value: "32768", label: t("aiSummary.debug.pickerOptions.ctx32k") },
    { value: "65536", label: t("aiSummary.debug.pickerOptions.ctx64k") },
  ];
}
function ctxToOption(n: number | null): CtxKey {
  if (n === 16384) return "16384";
  if (n === 32768) return "32768";
  if (n === 65536) return "65536";
  return "default";
}
/** "default" → null（让 overrides.ctxSize 留空，后端走 8K 默认）；其它 → 数值 */
function optionToCtx(v: CtxKey): number | null {
  return v === "default" ? null : parseInt(v, 10);
}

/** 估算当前引擎参数组合下的总 VRAM / RAM 占用（GB）。
 *
 * 简化模型——从 active_main 文件名抠出 "NB" 参数量，按 Q4 量化粗算：
 *   weights_GB    ≈ params × 0.55     （Q4_K_M 经验比例）
 *   kv_GB         ≈ params × 18 KB/token × ctx_total
 *   overhead_GB   ≈ 2.0               （vision encoder + workspace + activations）
 *
 * 误差 ±20%，用来给用户提前感知"这个组合是不是离 OOM 不远了"，
 * 不是精确值。撞了上限引擎会 fail-fast 报 cudaMalloc OOM 兜底。 */
function estimateVramGB(
  modelName: string,
  parallelSlots: number,
  ctxSize: number,
): { totalGB: number; weightsGB: number; kvGB: number; params: number } {
  // 文件名里像 "Qwen2.5-VL-3B" / "Qwen3VL-8B" / "gemma-4-4b" 都能匹配
  const m = modelName.match(/(\d+(?:\.\d+)?)\s*B/i);
  const params = m ? parseFloat(m[1]) : 4; // 找不到就按 4B 兜底
  const weightsGB = params * 0.55;
  const kvPerTokenKB = 18 * params;
  const totalCtx = ctxSize * Math.max(1, parallelSlots);
  const kvGB = (kvPerTokenKB * totalCtx) / 1024 / 1024;
  const overheadGB = 2;
  return {
    totalGB: weightsGB + kvGB + overheadGB,
    weightsGB,
    kvGB,
    params,
  };
}

const LOG_RING_SIZE = 200; // 防止整日跑事件流爆内存

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
function anchorDateStr(scope: DebugScope, offset: number): string {
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
function offsetLabel(scope: DebugScope, offset: number, t: TFunction): string {
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
function humanAccelLabel(platformId: string): string {
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

function nowHms(): string {
  const d = new Date();
  const hh = String(d.getHours()).padStart(2, "0");
  const mm = String(d.getMinutes()).padStart(2, "0");
  const ss = String(d.getSeconds()).padStart(2, "0");
  const ms = String(d.getMilliseconds()).padStart(3, "0");
  return `${hh}:${mm}:${ss}.${ms}`;
}

/** 把 phase + payload 浓缩成一行 log body 字符串。 */
function fmtPhaseBody(p: SummaryProgress): string {
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

/**
 * 调试 tab：本次只做前端骨架 —— 已接入的：
 *  - 引擎状态条（getEngineStatus）
 *  - 段下拉 + 开始 / 停止（复用 generateDaySummary / cancelDaySummary）
 *  - 逐图描述列表（getDayImageDescriptions + listen image_described）
 *  - 实时事件流 log（listen 全部 phase）
 *  - 段总结结果（getDaySummary + listen segment_done）
 *  - 导出 JSON（前端 Blob 打包）
 *
 * 待后端补的：
 *  - 单图重跑（行末按钮先 disabled）
 *  - Prompt 实际文本预览（折叠面板先 placeholder）
 *  - step 2 user prompt（同 placeholder）
 *  - 耗时 / token（描述行右侧留 "—"）
 */
export default function DebugTab() {
  const { t } = useTranslation();
  const { settings } = useSettings();
  const segments = settings?.ai.segments ?? [];
  const activeMain = settings?.ai.activeMain ?? "";
  const hasModel = activeMain.trim().length > 0;

  // Picker 选项随语言变化而重建——i18next 切语言会让 t 引用变更，触发 useMemo 重算
  const SCOPE_OPTIONS = useMemo(() => buildScopeOptions(t), [t]);
  const MAX_IMAGES_OPTIONS = useMemo(() => buildMaxImagesOptions(t), [t]);
  const SLOTS_OPTIONS = useMemo(() => buildSlotsOptions(t), [t]);
  const CTX_OPTIONS = useMemo(() => buildCtxOptions(t), [t]);

  // 工具：把 scope 翻译成"日报/周报/月报"对应文案，用于错误信息和占位
  const scopeName = (s: DebugScope) => t(`aiSummary.debug.scope.${s}`);

  const [dayOffset, setDayOffset] = useState(0);
  /** 顶部"调什么"——日报 / 周报 / 月报；后两个先占位等后端实现 */
  const [scope, setScope] = useState<DebugScope>("daily");
  const [generating, setGenerating] = useState(false);

  // 鼠标接近发光特效：跟 Today / DailyTab 同款
  const { ref: prevBtnRef } = useMouseGlow<HTMLButtonElement>();
  const { ref: pillRef } = useMouseGlow<HTMLButtonElement>();
  const { ref: nextBtnRef } = useMouseGlow<HTMLButtonElement>();
  const [enginePhase, setEnginePhase] = useState<string | null>(null);
  const [topError, setTopError] = useState<string | null>(null);

  /** 调试 tab 局部参数覆盖：初始从 settings.ai 拷贝，之后只在本 tab 改、不写 settings。
   *  跑总结时打包成 AiOverrides 传给 generate_day_summary，命令本次跑生效，跑完不留痕。 */
  const [debugMaxImages, setDebugMaxImages] = useState(30);
  const [debugExcluded, setDebugExcluded] = useState<string[]>([]);
  const [debugHashThreshold, setDebugHashThreshold] = useState(5);
  const [debugHashWindow, setDebugHashWindow] = useState(5);
  /** 启动级 override：null = 不传给后端（走 llama.cpp 默认）；改值会触发引擎重启。 */
  const [debugBatchSize, setDebugBatchSize] = useState<number | null>(null);
  const [debugParallelSlots, setDebugParallelSlots] = useState(1);
  const [debugCtxSize, setDebugCtxSize] = useState<number | null>(null);
  /** 调试 tab 局部 prompt 覆盖：默认值 = 当前 settings 的覆盖（非空）或内置默认。 */
  const [debugSysPrompt, setDebugSysPrompt] = useState("");
  const [debugImagePrompt, setDebugImagePrompt] = useState("");

  // settings 加载好后，把 ai 的几个字段拷成本地初值（只拷一次，之后不再跟 settings 同步）
  const initedRef = useRef(false);
  useEffect(() => {
    if (initedRef.current || !settings) return;
    initedRef.current = true;
    // 通过 picker 选项 round-trip 把 state snap 到 {15, 30, 100000} 之一——
    // 否则 settings.ai.maxImagesPerSegment 是 100 时 picker 会显示「30 张/段」
    // 但下发还是 100，用户感觉「我选 30 没生效」
    setDebugMaxImages(
      optionToMaxImages(maxImagesToOption(settings.ai.maxImagesPerSegment)),
    );
    setDebugExcluded(settings.ai.excludedCategories);
    setDebugHashThreshold(settings.ai.hashThreshold);
    setDebugHashWindow(settings.ai.hashWindowMinutes);
    // prompt：优先用 settings 覆盖，否则用内置默认；保证 textarea 一打开就有真实文本
    const lang = settings.ai.promptLanguage;
    const sysOverride = settings.ai.promptOverrides[
      lang === "en" ? "systemEn" : lang === "ja" ? "systemJa" : "systemZh"
    ];
    const imgOverride = settings.ai.imageDescribeOverrides?.[
      lang === "en" ? "systemEn" : lang === "ja" ? "systemJa" : "systemZh"
    ] ?? "";
    setDebugSysPrompt(sysOverride.trim() || DEFAULT_SYSTEM_PROMPTS[lang]);
    setDebugImagePrompt(imgOverride.trim() || DEFAULT_IMAGE_DESCRIBE_PROMPTS[lang]);
  }, [settings]);

  const [engine, setEngine] = useState<EngineStatus | null>(null);
  const [descs, setDescs] = useState<ImageDescriptionRow[]>([]);
  const [summaries, setSummaries] = useState<SegmentSummaryRow[]>([]);
  const [logs, setLogs] = useState<LogEntry[]>([]);
  /** llama-server 启动日志（GPU 加载情况、cuBLAS init 等）；点刷新拉一次 */
  const [engineLogs, setEngineLogs] = useState<string[]>([]);
  const [engineLogsBusy, setEngineLogsBusy] = useState(false);

  const refreshEngineLogs = async () => {
    setEngineLogsBusy(true);
    try {
      const lines = await api.getEngineLogs();
      setEngineLogs(lines);
    } catch (e) {
      console.warn("getEngineLogs 失败:", e);
    } finally {
      setEngineLogsBusy(false);
    }
  };

  // 锚定日期：daily=当天，weekly=该周一，monthly=该月 1 号；
  // 周报 / 月报命令未来传这个值。daily 之外的 scope 现在 onStart 会被拦掉，
  // 所以 anchor 暂时只用于 listen 的 date 比对（避免日报跑动时事件被误算成周报的）。
  const date = useMemo(() => anchorDateStr(scope, dayOffset), [scope, dayOffset]);

  // 进页 / 切日期：拉引擎状态 + 历史描述 + 段总结
  useEffect(() => {
    let cancelled = false;
    setDescs([]);
    setSummaries([]);
    setLogs([]);
    setEnginePhase(null);
    setTopError(null);

    Promise.all([
      api.getEngineStatus().catch((e) => {
        console.error("getEngineStatus 失败:", e);
        return null;
      }),
      api.getDayImageDescriptions(date, "debug").catch(() => [] as ImageDescriptionRow[]),
      api.getDaySummary(date, "debug").catch(() => [] as SegmentSummaryRow[]),
    ]).then(([eng, ds, sums]) => {
      if (cancelled) return;
      setEngine(eng);
      setDescs(ds);
      setSummaries(sums);
    });

    return () => {
      cancelled = true;
    };
  }, [date]);

  // listen 全局进度事件 —— 按 date 过滤
  const dateRef = useRef(date);
  dateRef.current = date;
  useEffect(() => {
    const p = listen<SummaryProgress>(SUMMARY_PROGRESS_EVENT, (ev) => {
      const ev_ = ev.payload;
      // 只接 debug source 的事件——日报跑时这个 listener 也会被广播到，
      // 不过滤会让调试 tab 看到 daily 数据
      if (ev_.source !== "debug") return;
      if (ev_.date !== dateRef.current) return;

      // 不管 phase 都进 log（rolling）
      const entry: LogEntry = {
        ts: nowHms(),
        phase: ev_.phase,
        body: fmtPhaseBody(ev_),
      };
      setLogs((prev) => {
        const next = [...prev, entry];
        if (next.length > LOG_RING_SIZE) next.splice(0, next.length - LOG_RING_SIZE);
        return next;
      });

      switch (ev_.phase) {
        case "engine_starting":
          setEnginePhase(ev_.message ?? tRef.current("aiSummary.debug.engineLoading"));
          break;
        case "segment_started":
          setEnginePhase(null);
          break;
        case "image_described": {
          // 实时往描述列表插一条 / 更新已有项
          if (ev_.segmentIdx == null || ev_.imageIndex == null) break;
          const row: ImageDescriptionRow = {
            source: "debug",
            localDate: ev_.date,
            segmentIdx: ev_.segmentIdx,
            imageIndex: ev_.imageIndex,
            screenshotPath: ev_.imagePath ?? "",
            description: ev_.imageDescription ?? "",
            model: activeMainRef.current,
            generatedAt: new Date().toISOString(),
            latencyMs: ev_.latencyMs,
            promptTokens: ev_.promptTokens,
            completionTokens: ev_.completionTokens,
          };
          setDescs((prev) => {
            const idx = prev.findIndex(
              (r) =>
                r.segmentIdx === row.segmentIdx &&
                r.imageIndex === row.imageIndex,
            );
            if (idx >= 0) {
              const next = prev.slice();
              next[idx] = row;
              return next;
            }
            return [...prev, row].sort(
              (a, b) =>
                a.segmentIdx - b.segmentIdx || a.imageIndex - b.imageIndex,
            );
          });
          break;
        }
        case "segment_done": {
          if (ev_.segmentIdx == null || !ev_.status) break;
          const seg = segmentsRef.current[ev_.segmentIdx];
          if (!seg) break;
          const row: SegmentSummaryRow = {
            source: "debug",
            localDate: ev_.date,
            segmentIdx: ev_.segmentIdx,
            label: seg.label,
            startHour: seg.startHour,
            endHour: seg.endHour,
            content: ev_.content ?? "",
            model: activeMainRef.current,
            status: ev_.status,
            error: ev_.message ?? null,
            generatedAt: new Date().toISOString(),
          };
          setSummaries((prev) => {
            const idx = prev.findIndex((r) => r.segmentIdx === row.segmentIdx);
            if (idx >= 0) {
              const next = prev.slice();
              next[idx] = row;
              return next;
            }
            return [...prev, row].sort((a, b) => a.segmentIdx - b.segmentIdx);
          });
          break;
        }
        case "all_done":
        case "cancelled":
          setGenerating(false);
          setEnginePhase(null);
          // 完成后刷一下引擎状态拿端口
          void api.getEngineStatus().then((s) => setEngine(s)).catch(() => {});
          break;
        case "error":
          setGenerating(false);
          setEnginePhase(null);
          setTopError(ev_.message ?? tRef.current("aiSummary.debug.errors.runFailed"));
          break;
      }
    });
    return () => {
      void p.then((unlisten) => unlisten());
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const segmentsRef = useRef(segments);
  segmentsRef.current = segments;
  const activeMainRef = useRef(activeMain);
  activeMainRef.current = activeMain;
  // listen 回调里要拿到最新的 t（切语言后才能用新语言显示），closure 里的 t 是旧的
  const tRef = useRef(t);
  tRef.current = t;


  const onStart = async () => {
    if (scope !== "daily") {
      setTopError(t("aiSummary.debug.errors.scopePending", { type: scopeName(scope) }));
      return;
    }
    if (!hasModel) {
      setTopError(t("aiSummary.debug.errors.noVisionModel"));
      return;
    }
    setGenerating(true);
    setTopError(null);
    try {
      // 调试模式 = force_refresh，清掉旧的重新跑一遍看完整流程
      // 调试 tab 把本地参数打包成 overrides 传给后端，本次生效不写 settings。
      // prompt 文本：跟内置默认一致 → 不传（让后端走默认逻辑）；不一致 → 传覆盖
      const lang = settings?.ai.promptLanguage ?? "zh";
      const sysPromptDefault = DEFAULT_SYSTEM_PROMPTS[lang];
      const imgPromptDefault = DEFAULT_IMAGE_DESCRIBE_PROMPTS[lang];
      const overrides: AiOverrides = {
        excludedCategories: debugExcluded,
        maxImagesPerSegment: debugMaxImages,
        hashThreshold: debugHashThreshold,
        hashWindowMinutes: debugHashWindow,
        systemPrompt:
          debugSysPrompt.trim() === sysPromptDefault.trim() ? "" : debugSysPrompt,
        imageDescribePrompt:
          debugImagePrompt.trim() === imgPromptDefault.trim()
            ? ""
            : debugImagePrompt,
        // null / 1 → 不传，让后端走默认；非默认值才打包，触发 stop+start with overrides
        ...(debugBatchSize != null ? { batchSize: debugBatchSize } : {}),
        ...(debugParallelSlots > 1 ? { parallelSlots: debugParallelSlots } : {}),
        ...(debugCtxSize != null ? { ctxSize: debugCtxSize } : {}),
      };
      await api.generateDaySummary(date, true, null, overrides, "debug");
    } catch (e) {
      const msg = typeof e === "string" ? e : String(e);
      setTopError(msg);
      setGenerating(false);
    }
  };

  const onStop = async () => {
    try {
      await api.cancelDaySummary();
    } catch (e) {
      console.warn("cancel 失败:", e);
    }
  };

  const onExport = () => {
    const payload = {
      exportedAt: new Date().toISOString(),
      date,
      activeModel: activeMain,
      engine,
      segments,
      summaries,
      imageDescriptions: descs,
      logs,
    };
    const blob = new Blob([JSON.stringify(payload, null, 2)], {
      type: "application/json",
    });
    const url = URL.createObjectURL(blob);
    const a = document.createElement("a");
    a.href = url;
    a.download = `hindsight-debug-${date}.json`;
    a.click();
    setTimeout(() => URL.revokeObjectURL(url), 1000);
  };

  // 周报 / 月报后端没实现，描述列表和总结都按 scope 切：非 daily 时清空显示
  const visibleDescs = scope === "daily" ? descs : [];
  const visibleSummaries = scope === "daily" ? summaries : [];

  return (
    <div className={styles.wrap}>
      {/* —— 顶部控件行：报告类型 → 日期 → 开始 → 导出 —— */}
      <div className={styles.header}>
        {/* 调试范围下拉：日报 / 周报 / 月报。样式与 Today 页 DevicePicker 一致。 */}
        <SimplePicker<DebugScope>
          value={scope}
          options={SCOPE_OPTIONS}
          onChange={(next) => {
            setScope(next);
            setDayOffset(0); // 切 scope 时回到"当前周期"
          }}
          disabled={generating}
        />

        <div className={styles.dateNav}>
          <button
            ref={prevBtnRef}
            type="button"
            className={`${styles.navBtn} glow`}
            onClick={() => setDayOffset((v) => v - 1)}
            disabled={generating}
            aria-label={t(
              scope === "daily"
                ? "aiSummary.debug.dateNav.prevDayAria"
                : scope === "weekly"
                  ? "aiSummary.debug.dateNav.prevWeekAria"
                  : "aiSummary.debug.dateNav.prevMonthAria",
            )}
          >
            <ChevronLeft size={14} strokeWidth={1.75} />
          </button>
          <button
            ref={pillRef}
            type="button"
            className={`${styles.dayPill} glow`}
            onClick={() => setDayOffset(0)}
            disabled={generating || dayOffset === 0}
            title={t(
              scope === "daily"
                ? "aiSummary.debug.dateNav.todayBack"
                : scope === "weekly"
                  ? "aiSummary.debug.dateNav.thisWeekBack"
                  : "aiSummary.debug.dateNav.thisMonthBack",
            )}
          >
            {offsetLabel(scope, dayOffset, t)}
          </button>
          <button
            ref={nextBtnRef}
            type="button"
            className={`${styles.navBtn} glow`}
            onClick={() => setDayOffset((v) => v + 1)}
            disabled={generating || dayOffset >= 0}
            aria-label={t(
              scope === "daily"
                ? "aiSummary.debug.dateNav.nextDayAria"
                : scope === "weekly"
                  ? "aiSummary.debug.dateNav.nextWeekAria"
                  : "aiSummary.debug.dateNav.nextMonthAria",
            )}
          >
            <ChevronRight size={14} strokeWidth={1.75} />
          </button>
        </div>

        {/* 图/段：3 档下拉，绑调试本地 state（不写 settings）。
            "无限制" = 100000；跑总结时打包进 overrides 传给后端，本次生效不留痕 */}
        <span className={styles.pickerWithInfo}>
          <SimplePicker<MaxImagesKey>
            value={maxImagesToOption(debugMaxImages)}
            options={MAX_IMAGES_OPTIONS}
            onChange={(next) => setDebugMaxImages(optionToMaxImages(next))}
            disabled={generating || !settings}
          />
          <span
            className={styles.infoIconWrap}
            title={t("aiSummary.debug.maxImagesInfo.tooltip")}
            tabIndex={0}
            aria-label={t("aiSummary.debug.maxImagesInfo.aria")}
          >
            <Info size={13} strokeWidth={1.85} />
          </span>
        </span>

        {generating ? (
          <button
            type="button"
            className={styles.stopBtn}
            onClick={() => void onStop()}
          >
            <Square size={14} strokeWidth={2} />
            {t("aiSummary.debug.actions.stop")}
          </button>
        ) : (
          <button
            type="button"
            className={styles.startBtn}
            onClick={() => void onStart()}
            disabled={!hasModel || scope !== "daily"}
            title={
              scope !== "daily"
                ? t("aiSummary.debug.actions.startTooltipPending", { type: scopeName(scope) })
                : hasModel
                  ? t("aiSummary.debug.actions.startTooltipReady")
                  : t("aiSummary.debug.actions.startTooltipNoModel")
            }
          >
            <Play size={14} strokeWidth={2} />
            {t("aiSummary.debug.actions.start")}
          </button>
        )}

        <button
          type="button"
          className={styles.deleteBtn}
          onClick={async () => {
            if (generating) return;
            if (
              !confirm(
                t("aiSummary.debug.actions.deleteConfirm", { date }),
              )
            )
              return;
            try {
              await api.clearDaySummary(date, "debug");
              setDescs([]);
              setSummaries([]);
              setLogs([]);
            } catch (e) {
              setTopError(typeof e === "string" ? e : String(e));
            }
          }}
          disabled={
            generating || (descs.length === 0 && summaries.length === 0)
          }
          title={t("aiSummary.debug.actions.deleteTooltip")}
        >
          <Trash2 size={13} strokeWidth={2} />
          {t("aiSummary.debug.actions.delete")}
        </button>

        <button
          type="button"
          className={styles.exportBtn}
          onClick={onExport}
          disabled={
            descs.length === 0 && summaries.length === 0 && logs.length === 0
          }
          title={t("aiSummary.debug.actions.exportTooltip")}
        >
          <Download size={13} strokeWidth={2} />
          {t("aiSummary.debug.actions.exportJson")}
        </button>
      </div>

      {/* —— 引擎状态条 —— */}
      <EngineBar engine={engine} />

      {/* —— 调试参数：完全复用 AI 设置的 Section + Row 样式，
          但绑定本地 state（不写全局 settings），跑总结时打包成 overrides 传后端 —— */}
      <Section
        title={t("aiSummary.debug.filter.title")}
        icon={Filter}
        description={t("aiSummary.debug.filter.description")}
      >
        <Row
          label={t("aiSummary.debug.filter.categoriesLabel")}
          labelHint={t("aiSummary.debug.filter.categoriesHint")}
          block
        >
          <CategoryChipMultiSelect
            selectedIds={debugExcluded}
            onChange={setDebugExcluded}
          />
        </Row>
      </Section>

      <Section
        title={t("aiSummary.debug.frame.title")}
        icon={ImageIcon}
        description={t("aiSummary.debug.frame.description")}
      >
        <Row
          label={t("aiSummary.debug.frame.hashThresholdLabel")}
          labelHint={t("aiSummary.debug.frame.hashThresholdHint")}
        >
          <Slider
            value={debugHashThreshold}
            onChange={setDebugHashThreshold}
            min={0}
            max={32}
            step={1}
          />
        </Row>
        <Row
          label={t("aiSummary.debug.frame.hashWindowLabel")}
          labelHint={t("aiSummary.debug.frame.hashWindowHint")}
        >
          <Slider
            value={debugHashWindow}
            onChange={setDebugHashWindow}
            min={0}
            max={30}
            step={1}
            suffix={t("aiSummary.debug.frame.hashWindowSuffix")}
          />
        </Row>
      </Section>

      {/* —— 引擎参数（启动时）：改值会重启引擎，仅本调试窗口生效。
          3 个 picker 自带类目前缀（"Batch 512"/"并发 4 路"/"ctx 16K/槽"）
          直接 inline 排，跟顶部「日报 / 今天 / 30 张/段」那排同款。
          末尾挂一行 KV cache 估算，让用户在点开始前就感知"这组合会不会 OOM"。 */}
      <Section
        title={t("aiSummary.debug.engine.title")}
        icon={Server}
        description={t("aiSummary.debug.engine.description")}
      >
        <div className={styles.engineParamRow}>
          <SimplePicker<BatchKey>
            value={batchToOption(debugBatchSize)}
            options={BATCH_OPTIONS}
            onChange={(next) => setDebugBatchSize(optionToBatch(next))}
            disabled={generating}
          />
          <SimplePicker<SlotsKey>
            value={slotsToOption(debugParallelSlots)}
            options={SLOTS_OPTIONS}
            onChange={(next) => setDebugParallelSlots(optionToSlots(next))}
            disabled={generating}
          />
          <SimplePicker<CtxKey>
            value={ctxToOption(debugCtxSize)}
            options={CTX_OPTIONS}
            onChange={(next) => setDebugCtxSize(optionToCtx(next))}
            disabled={generating}
          />
        </div>
        <VramEstimateLine
          modelName={activeMain}
          parallelSlots={debugParallelSlots}
          ctxSize={debugCtxSize ?? 8192}
        />
      </Section>

      {/* —— 错误条 / 冷启动提示 —— */}
      {topError ? (
        <div className={styles.errorBar}>
          <AlertTriangle size={14} strokeWidth={2.2} />
          <span>{topError}</span>
        </div>
      ) : null}
      {enginePhase ? (
        <div className={styles.engineHint}>
          <Loader2 size={14} className={styles["spin"]} />
          <span>{enginePhase}</span>
        </div>
      ) : null}

      {/* —— 非日报 scope 的占位 —— */}
      {scope !== "daily" ? (
        <div className={styles.placeholder}>
          {t("aiSummary.debug.scopePlaceholder", { type: scopeName(scope) })}
        </div>
      ) : null}

      {/* —— 图片描述提示词：独立 Section box；textarea 默认收起，hover 该卡片展开 —— */}
      <div className={styles.promptCollapseWrap}>
        <Section
          title={t("aiSummary.debug.imagePrompt.title")}
          icon={MessageSquareText}
          description={t("aiSummary.debug.imagePrompt.description")}
        >
          <Row label={t("aiSummary.debug.imagePrompt.label")} block>
            <div className={styles.collapsibleWrap}>
              <textarea
                className={`${styles.debugPromptTextarea} ${styles.collapsibleTextarea}`}
                value={debugImagePrompt}
                onChange={(e) => setDebugImagePrompt(e.target.value)}
                rows={8}
                spellCheck={false}
              />
              <div className={styles.collapseFade} aria-hidden />
            </div>
          </Row>
        </Section>
      </div>

      {/* —— 时间段总结提示词：独立 Section box，跟上面互不影响 —— */}
      <div className={styles.promptCollapseWrap}>
        <Section
          title={t("aiSummary.debug.segPrompt.title")}
          icon={MessageSquareText}
          description={t("aiSummary.debug.segPrompt.description")}
        >
          <Row label={t("aiSummary.debug.segPrompt.label")} block>
            <div className={styles.collapsibleWrap}>
              <textarea
                className={`${styles.debugPromptTextarea} ${styles.collapsibleTextarea}`}
                value={debugSysPrompt}
                onChange={(e) => setDebugSysPrompt(e.target.value)}
                rows={10}
                spellCheck={false}
              />
              <div className={styles.collapseFade} aria-hidden />
            </div>
          </Row>
        </Section>
      </div>

      {/* —— 逐图描述：包到 Section box，跟其他卡片视觉一致 —— */}
      <Section
        title={t("aiSummary.debug.perImage.title")}
        icon={ImageIcon}
        description={
          t("aiSummary.debug.perImage.descriptionBase") +
          (visibleDescs.length > 0
            ? t("aiSummary.debug.perImage.countSuffix", { count: visibleDescs.length })
            : "") +
          t("aiSummary.debug.perImage.descriptionSuffix")
        }
      >
        {visibleDescs.length === 0 ? (
          <div className={styles.descListEmpty}>
            {t("aiSummary.debug.perImage.empty")}
          </div>
        ) : (
          <div className={styles.descListInner}>
            {visibleDescs.map((d) => (
              <DescItem
                key={`${d.segmentIdx}-${d.imageIndex}`}
                row={d}
                segmentLabel={segments[d.segmentIdx]?.label}
                segmentColor={segments[d.segmentIdx]?.color}
                onOpenError={setTopError}
                onRetry={async () => {
                  if (generating) return;
                  try {
                    await api.retrySingleImageDescription(
                      date,
                      d.segmentIdx,
                      d.imageIndex,
                      {
                        excludedCategories: debugExcluded,
                        maxImagesPerSegment: debugMaxImages,
                        hashThreshold: debugHashThreshold,
                        hashWindowMinutes: debugHashWindow,
                        systemPrompt:
                          debugSysPrompt.trim() ===
                          (DEFAULT_SYSTEM_PROMPTS[
                            settings?.ai.promptLanguage ?? "zh"
                          ]?.trim() ?? "")
                            ? ""
                            : debugSysPrompt,
                        imageDescribePrompt:
                          debugImagePrompt.trim() ===
                          (DEFAULT_IMAGE_DESCRIBE_PROMPTS[
                            settings?.ai.promptLanguage ?? "zh"
                          ]?.trim() ?? "")
                            ? ""
                            : debugImagePrompt,
                        ...(debugBatchSize != null
                          ? { batchSize: debugBatchSize }
                          : {}),
                        ...(debugParallelSlots > 1
                          ? { parallelSlots: debugParallelSlots }
                          : {}),
                        ...(debugCtxSize != null
                          ? { ctxSize: debugCtxSize }
                          : {}),
                      },
                      "debug",
                    );
                  } catch (e) {
                    setTopError(typeof e === "string" ? e : String(e));
                  }
                }}
                retryDisabled={generating}
              />
            ))}
          </div>
        )}
      </Section>


      {/* —— 段总结结果：用 Section 跟「逐图描述」视觉对齐，常开 + 滚动 —— */}
      <Section
        title={t("aiSummary.debug.segments.title")}
        icon={Newspaper}
        description={
          t("aiSummary.debug.segments.descriptionBase") +
          (visibleSummaries.length > 0
            ? t("aiSummary.debug.segments.countSuffix", { count: visibleSummaries.length })
            : "") +
          t("aiSummary.debug.segments.descriptionSuffix")
        }
      >
        {visibleSummaries.length === 0 ? (
          <div className={styles.summaryEmpty}>
            {t("aiSummary.debug.segments.empty")}
          </div>
        ) : (
          <div className={styles.panelOpen}>
            {visibleSummaries.map((s) => {
              // 段 chip 背景色：跟 DescItem 一样从 segment.color 取，没有则中性灰
              const seg = segments[s.segmentIdx];
              const chipBg =
                seg?.color && seg.color.trim().length > 0
                  ? seg.color
                  : "#cbd5e1";
              const chipColor = isLightHex(chipBg) ? "#3a3f55" : "#fff";
              // 状态徽章：ok 绿 / error 红 / skipped 灰
              const statusClass =
                s.status === "ok"
                  ? styles.summaryStatusOk
                  : s.status === "skipped_no_screenshots"
                    ? styles.summaryStatusSkipped
                    : styles.summaryStatusError;
              const statusText =
                s.status === "ok"
                  ? "ok"
                  : s.status === "skipped_no_screenshots"
                    ? "skipped"
                    : "error";
              return (
                <div key={s.segmentIdx} className={styles.summaryBox}>
                  <div className={styles.summaryHead}>
                    <span
                      className={styles.summaryChip}
                      style={{ background: chipBg, color: chipColor }}
                      title={t("aiSummary.debug.segments.chipTitle", { idx: s.segmentIdx })}
                    >
                      {s.label}
                    </span>
                    <span className={styles.summaryTimeRange}>
                      <Clock size={12} strokeWidth={2.2} />
                      {String(s.startHour).padStart(2, "0")}:00 –{" "}
                      {String(s.endHour).padStart(2, "0")}:00
                    </span>
                    <span className={`${styles.summaryStatus} ${statusClass}`}>
                      {statusText}
                    </span>
                  </div>
                  <div className={styles.summaryText}>
                    {s.content ||
                      (s.status === "skipped_no_screenshots"
                        ? t("aiSummary.debug.segments.skippedFallback")
                        : s.error || t("aiSummary.debug.segments.emptyFallback"))}
                  </div>
                </div>
              );
            })}
          </div>
        )}
      </Section>

      {/* —— 实时事件流 —— */}
      <div className={styles.panelWrap}>
        <span className={styles.panelLabel}>{t("aiSummary.debug.events.label")}</span>
        <div className={styles.panel}>
          <div className={styles.logBox}>
            {logs.length === 0 ? (
              <div className={styles.logEmpty}>{t("aiSummary.debug.events.empty")}</div>
            ) : (
              logs.map((entry, i) => (
                <div key={i} className={styles.logLine}>
                  <span className={styles.logTime}>{entry.ts}</span>
                  <span
                    className={`${styles.logPhase} ${
                      entry.phase === "error"
                        ? styles.logPhaseError
                        : entry.phase === "all_done" || entry.phase === "segment_done"
                          ? styles.logPhaseDone
                          : ""
                    }`}
                  >
                    {entry.phase}
                  </span>
                  <span className={styles.logBody}>{entry.body}</span>
                </div>
              ))
            )}
          </div>
        </div>
      </div>

      {/* —— 引擎启动日志（llama-server stderr / stdout）——
          诊断 GPU 加载情况：找 "offloaded XX/YY layers to GPU" / "cuBLAS init" 等行。 */}
      <div className={styles.panelWrap}>
        <span className={styles.panelLabel}>
          {t("aiSummary.debug.engineLogs.label")}
          <button
            type="button"
            className={styles.engineLogsRefreshBtn}
            onClick={() => void refreshEngineLogs()}
            disabled={engineLogsBusy}
            title={t("aiSummary.debug.engineLogs.refreshTooltip")}
          >
            {engineLogsBusy
              ? t("aiSummary.debug.engineLogs.refreshing")
              : t("aiSummary.debug.engineLogs.refresh")}
          </button>
        </span>
        <div className={styles.panel}>
          <div className={styles.logBox}>
            {engineLogs.length === 0 ? (
              <div className={styles.logEmpty}>
                {t("aiSummary.debug.engineLogs.emptyPrefix")}
                <code>offloaded XX/YY layers to GPU</code>
                {t("aiSummary.debug.engineLogs.emptySuffix")}
              </div>
            ) : (
              engineLogs.map((line, i) => (
                <div key={i} className={styles.logLine}>
                  <span className={styles.logBody}>{line}</span>
                </div>
              ))
            )}
          </div>
        </div>
      </div>
    </div>
  );
}

/** 「引擎参数」Section 末尾的估算行——基于当前 picker 组合 + active model
 *  文件名抠出的参数量，粗估总占用。颜色按风险分档：
 *   < 16 GB 灰、16-24 橙、> 24 红（5090 32GB 留 ~8 GB margin 给系统 / encoder）
 */
function VramEstimateLine({
  modelName,
  parallelSlots,
  ctxSize,
}: {
  modelName: string;
  parallelSlots: number;
  ctxSize: number;
}) {
  const { t } = useTranslation();
  if (!modelName.trim()) {
    return null;
  }
  const est = estimateVramGB(modelName, parallelSlots, ctxSize);
  let levelClass = styles.vramEstOk;
  if (est.totalGB > 24) levelClass = styles.vramEstDanger;
  else if (est.totalGB > 16) levelClass = styles.vramEstWarn;
  return (
    <div className={`${styles.vramEst} ${levelClass}`}>
      <span className={styles.vramEstLabel}>{t("aiSummary.debug.vram.label")}</span>
      <span className={styles.vramEstValue}>
        {t("aiSummary.debug.vram.value", { total: est.totalGB.toFixed(1) })}
      </span>
      <span className={styles.vramEstBreakdown}>
        {t("aiSummary.debug.vram.breakdown", {
          weights: est.weightsGB.toFixed(1),
          kv: est.kvGB.toFixed(1),
          params: est.params,
          ctxK: (ctxSize / 1024) | 0,
          slots: parallelSlots,
        })}
      </span>
    </div>
  );
}

/** 引擎状态条：端口 / 模型 / ctx / 状态指示 dot。 */
function EngineBar({ engine }: { engine: EngineStatus | null }) {
  const { t } = useTranslation();
  if (!engine) {
    return (
      <div className={styles.engineBar}>
        <span className={styles.engineDot} />
        <span>{t("aiSummary.debug.engineBar.loading")}</span>
      </div>
    );
  }
  const rt = engine.runtime;
  const dotClass =
    rt.state === "running"
      ? styles.engineDotRunning
      : rt.state === "starting"
        ? styles.engineDotStarting
        : rt.state === "error"
          ? styles.engineDotError
          : "";
  const versionStr = engine.installed
    ? engine.installedVersion ?? engine.currentPin
    : t("aiSummary.debug.engineBar.notInstalled");
  return (
    <div className={styles.engineBar}>
      <span className={`${styles.engineDot} ${dotClass}`} />
      <span>
        {t("aiSummary.debug.engineBar.port")}
        <span className={styles.engineMetaStrong}>
          {rt.state === "running" && rt.port != null ? `:${rt.port}` : "—"}
        </span>
      </span>
      <span className={styles.engineSep}>·</span>
      <span>
        {t("aiSummary.debug.engineBar.version")}
        <span className={styles.engineMetaStrong}>{versionStr}</span>
      </span>
      <span className={styles.engineSep}>·</span>
      <span>
        {t("aiSummary.debug.engineBar.accel")}
        <span
          className={styles.engineMetaStrong}
          title={t("aiSummary.debug.engineBar.accelTitle", { id: engine.platformId })}
        >
          {humanAccelLabel(engine.platformId)}
        </span>
      </span>
      <span className={styles.engineSep}>·</span>
      <span>
        {t("aiSummary.debug.engineBar.state")}
        <span className={styles.engineMetaStrong}>{rt.state}</span>
      </span>
      {rt.error ? (
        <>
          <span className={styles.engineSep}>·</span>
          <span style={{ color: "#dc2626" }}>
            {t("aiSummary.debug.engineBar.error")}
            {rt.error}
          </span>
        </>
      ) : null}
    </div>
  );
}

/** 单条逐图描述项。
 *  - 段标识背景色用 settings.ai.segments[idx].color（用户在「时段划分」里配的色）
 *  - 文件名 click → openPath 用系统默认查看器预览原图
 *  - 耗时 / token 来自 ai_image_descriptions 行 + image_described 事件
 *  - 重跑按钮调 api.retrySingleImageDescription */
function DescItem({
  row,
  segmentLabel,
  segmentColor,
  onRetry,
  retryDisabled,
  onOpenError,
}: {
  row: ImageDescriptionRow;
  segmentLabel?: string;
  segmentColor?: string;
  onRetry: () => void;
  retryDisabled: boolean;
  /** 打开图片失败时上报错误给父组件展示（顶部 errorBar） */
  onOpenError: (msg: string) => void;
}) {
  const { t } = useTranslation();
  const fileName = row.screenshotPath.split(/[\\/]/).pop() ?? row.screenshotPath;
  // 段背景色优先用 user-config（segments[idx].color），空时回到中性灰
  const chipBg = segmentColor && segmentColor.trim().length > 0
    ? segmentColor
    : "#cbd5e1";
  // 简单 perceived luminance 决定文字明暗：浅底用深字，深底用白字
  const chipColor = isLightHex(chipBg) ? "#3a3f55" : "#fff";

  // 耗时 / token 文本：null 时显示 "—"，让排版稳定
  const latencyStr = row.latencyMs != null ? `${row.latencyMs} ms` : "—";
  const tokenStr =
    row.promptTokens != null || row.completionTokens != null
      ? `${row.promptTokens ?? "—"} / ${row.completionTokens ?? "—"} t`
      : "— / — t";

  return (
    <div className={styles.descItem}>
      <div className={styles.descMeta}>
        <span
          className={styles.descIndex}
          style={{ background: chipBg, color: chipColor }}
          title={t("aiSummary.debug.perImage.chipTitle", {
            seg: row.segmentIdx,
            img: row.imageIndex,
          })}
        >
          {segmentLabel ?? t("aiSummary.debug.perImage.segFallback", { idx: row.segmentIdx })}
          {t("aiSummary.debug.perImage.imageNoSuffix", { n: row.imageIndex + 1 })}
        </span>
        <button
          type="button"
          className={styles.descPath}
          title={t("aiSummary.debug.perImage.pathTooltip", { path: row.screenshotPath })}
          onClick={() => {
            // 先试系统默认查看器；失败 fallback 到资源管理器选中文件
            // ——后者用 opener:default 自带的 allow-reveal-item-in-dir 权限，
            // 不依赖 capability 是否新加了 allow-open-path
            void (async () => {
              try {
                await openPath(row.screenshotPath);
                return;
              } catch (e) {
                console.warn("openPath 失败，fallback 到 reveal:", e);
              }
              try {
                await revealItemInDir(row.screenshotPath);
              } catch (e2) {
                const msg = e2 instanceof Error ? e2.message : String(e2);
                onOpenError(t("aiSummary.debug.perImage.openImageError", { msg }));
              }
            })();
          }}
        >
          {fileName}
        </button>
        <span
          className={styles.descStat}
          title={t("aiSummary.debug.perImage.statTooltip")}
        >
          {latencyStr} · {tokenStr}
        </span>
        <button
          type="button"
          className={styles.retryImg}
          onClick={onRetry}
          disabled={retryDisabled}
          title={
            retryDisabled
              ? t("aiSummary.debug.perImage.retryTooltipBusy")
              : t("aiSummary.debug.perImage.retryTooltipReady")
          }
        >
          <RotateCcw size={11} strokeWidth={2.2} />
          {t("aiSummary.debug.perImage.retry")}
        </button>
      </div>
      <div className={styles.descText}>
        {row.description || t("aiSummary.debug.perImage.descEmpty")}
      </div>
    </div>
  );
}

/** 用 perceived luminance 判 hex 是不是浅色（chip 文字明暗用） */
function isLightHex(hex: string): boolean {
  const m = hex.match(/^#([0-9a-f]{3}|[0-9a-f]{6})$/i);
  if (!m) return true;
  let h = m[1];
  if (h.length === 3) h = h.split("").map((c) => c + c).join("");
  const r = parseInt(h.slice(0, 2), 16);
  const g = parseInt(h.slice(2, 4), 16);
  const b = parseInt(h.slice(4, 6), 16);
  return (0.299 * r + 0.587 * g + 0.114 * b) / 255 > 0.6;
}
