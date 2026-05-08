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
  Image as ImageIcon,
  Info,
  Loader2,
  MessageSquareText,
  Newspaper,
  Play,
  RotateCcw,
  Square,
  Trash2,
} from "lucide-react";
import {
  api,
  SUMMARY_PROGRESS_EVENT,
  type AiOverrides,
  type AiSegment,
  type EngineStatus,
  type ImageDescriptionRow,
  type SegmentSummaryRow,
  type SummaryProgress,
} from "../../../api/hindsight";
import { useSettings } from "../../../state/settings";
import { useDebugState } from "../DebugStateContext";
import { resolveSegmentChip } from "../../../utils/segmentColor";
import { extractScreenshotTime } from "../../../utils/screenshotTime";
import { SimplePicker } from "../../../components/SimplePicker/SimplePicker";
import { Row } from "../../../components/FormLayout/Row";
import { Section } from "../../../components/FormLayout/Section";
import {
  DEFAULT_IMAGE_DESCRIBE_PROMPTS,
  DEFAULT_SYSTEM_PROMPTS,
} from "../../../lib/aiPrompts";
import { logError, logWarn } from "../../../lib/logger";
import {
  buildMaxImagesOptions,
  maxImagesToOption,
  optionToMaxImages,
  type MaxImagesKey,
} from "./debugTabOptions";
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

  // 调试参数 state 来自 Context，跟 DebugSettingsTab 共享同一份。
  const {
    debugMaxImages,
    setDebugMaxImages,
    debugExcluded,
    debugHashThreshold,
    debugHashWindow,
    debugDescribeBatchSize,
    debugDescribeParallelSlots,
    debugDescribeCtxSize,
    debugSummaryBatchSize,
    debugSummaryParallelSlots,
    debugSummaryCtxSize,
    debugSysPrompt,
    setDebugSysPrompt,
    debugImagePrompt,
    setDebugImagePrompt,
    debugExternalEnabled,
  } = useDebugState();

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
      logWarn("debug.getEngineLogs", e);
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
        logError("debug.getEngineStatus", e);
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
  }, []);

  const segmentsRef = useRef(segments);
  segmentsRef.current = segments;
  const activeMainRef = useRef(activeMain);
  activeMainRef.current = activeMain;
  // listen 回调里要拿到最新的 t（切语言后才能用新语言显示），closure 里的 t 是旧的
  const tRef = useRef(t);
  tRef.current = t;


  /** 跑 debug 总结。
   *  - mode="full"（默认）：完整 step1+step2
   *  - mode="step1"：只逐图描述（「逐图描述」Section header 按钮触发）
   *  - mode="step2"：只段总结（「段总结结果」Section header 按钮触发，从 DB 读已存图描述） */
  const onStart = async (mode: "full" | "step1" | "step2" = "full") => {
    const step1Only = mode === "step1";
    const step2Only = mode === "step2";
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
        // 双套引擎参数——null / 1 = 不传，让后端 fallback 到 settings.ai 默认；
        // 非默认值才打包到对应的 describe* / summary* 字段，触发 stop+start with overrides
        ...(debugDescribeBatchSize != null ? { describeBatchSize: debugDescribeBatchSize } : {}),
        ...(debugDescribeParallelSlots > 1 ? { describeParallelSlots: debugDescribeParallelSlots } : {}),
        ...(debugDescribeCtxSize != null ? { describeCtxSize: debugDescribeCtxSize } : {}),
        ...(debugSummaryBatchSize != null ? { summaryBatchSize: debugSummaryBatchSize } : {}),
        ...(debugSummaryParallelSlots > 1 ? { summaryParallelSlots: debugSummaryParallelSlots } : {}),
        ...(debugSummaryCtxSize != null ? { summaryCtxSize: debugSummaryCtxSize } : {}),
        // 跟 settings 全局值不同才传——一致的话留 undefined 让后端走 settings.ai.externalEnabled
        ...(debugExternalEnabled !== (settings?.ai.externalEnabled ?? false)
          ? { externalEnabled: debugExternalEnabled }
          : {}),
      };
      await api.generateDaySummary(date, true, null, overrides, "debug", step1Only, step2Only);
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
      logWarn("debug.cancel", e);
    }
  };

  /** 把 markdown 文本以 .md 文件下载——浏览器原生 download，落到系统 Downloads 目录。 */
  const downloadMarkdown = (md: string, filename: string) => {
    const blob = new Blob([md], { type: "text/markdown;charset=utf-8" });
    const url = URL.createObjectURL(blob);
    const a = document.createElement("a");
    a.href = url;
    a.download = filename;
    a.click();
    URL.revokeObjectURL(url);
  };

  /** 导出当天所有逐图描述：每段一个 H2，段内每张图按时间标签 + 文件名 + 描述正文。 */
  const onExportDescriptionsMd = () => {
    if (descs.length === 0) return;
    // 按 segmentIdx → imageIndex 分组排序
    const bySeg = new Map<number, ImageDescriptionRow[]>();
    descs.forEach((d) => {
      const arr = bySeg.get(d.segmentIdx) ?? [];
      arr.push(d);
      bySeg.set(d.segmentIdx, arr);
    });
    bySeg.forEach((arr) => arr.sort((a, b) => a.imageIndex - b.imageIndex));

    const lines: string[] = ["---"];
    lines.push(`title: Hindsight image descriptions · ${date}`);
    lines.push(`date: ${date}`);
    lines.push(`source: debug`);
    lines.push(`count: ${descs.length}`);
    lines.push("---", "");
    lines.push(`# Image descriptions · ${date}`, "");

    const sortedSegs = Array.from(bySeg.keys()).sort((a, b) => a - b);
    sortedSegs.forEach((segIdx, i) => {
      const seg = segments[segIdx];
      const label =
        seg?.label ?? t("aiSummary.debug.perImage.segFallback", { idx: segIdx });
      const range = seg
        ? ` · ${String(seg.startHour).padStart(2, "0")}:00 – ${String(seg.endHour).padStart(2, "0")}:00`
        : "";
      lines.push(`## ${label}${range}`, "");
      const items = bySeg.get(segIdx)!;
      items.forEach((row) => {
        const time = extractScreenshotTime(row.screenshotPath);
        const file = row.screenshotPath.split(/[\\/]/).pop() ?? row.screenshotPath;
        lines.push(`### ${time} · \`${file}\``, "");
        lines.push(row.description.trim() || "(empty)", "");
      });
      if (i < sortedSegs.length - 1) lines.push("---", "");
    });

    downloadMarkdown(lines.join("\n"), `hindsight-debug-descriptions-${date}.md`);
  };

  /** 导出当天所有段总结：跟 DailyTab 的导出格式对齐——frontmatter + 每段 H2 + 正文。 */
  const onExportSummariesMd = () => {
    if (summaries.length === 0) return;
    const sorted = [...summaries].sort((a, b) => a.segmentIdx - b.segmentIdx);
    let okCount = 0,
      skipCount = 0,
      errCount = 0;
    let modelName = "";
    let latestGeneratedAt = "";
    sorted.forEach((row) => {
      if (row.status === "ok") okCount += 1;
      else if (row.status === "skipped_no_screenshots") skipCount += 1;
      else errCount += 1;
      if (row.model) modelName = row.model;
      if (row.generatedAt && (!latestGeneratedAt || row.generatedAt > latestGeneratedAt)) {
        latestGeneratedAt = row.generatedAt;
      }
    });

    const lines: string[] = ["---"];
    lines.push(`title: Hindsight segment summaries · ${date}`);
    lines.push(`date: ${date}`);
    lines.push(`source: debug`);
    if (latestGeneratedAt) lines.push(`generated_at: ${latestGeneratedAt}`);
    if (modelName) lines.push(`model: ${modelName}`);
    lines.push(`segments: ${sorted.length}`);
    lines.push(`status: ${okCount} ok / ${skipCount} skipped / ${errCount} error`);
    lines.push("---", "");
    lines.push(`# Segment summaries · ${date}`, "");

    sorted.forEach((row, i) => {
      const range = `${String(row.startHour).padStart(2, "0")}:00 – ${String(row.endHour).padStart(2, "0")}:00`;
      lines.push(`## ${row.label} · ${range}`, "");
      if (row.status === "ok") {
        lines.push(row.content?.trim() || "(empty)", "");
      } else if (row.status === "skipped_no_screenshots") {
        lines.push("_skipped (no screenshots in this segment)_", "");
      } else {
        lines.push("_error_", "");
        if (row.error) lines.push(`> ${row.error.replace(/\n/g, "\n> ")}`, "");
      }
      if (i < sorted.length - 1) lines.push("---", "");
    });

    downloadMarkdown(lines.join("\n"), `hindsight-debug-summaries-${date}.md`);
  };

  // 周报 / 月报后端没实现，描述列表和总结都按 scope 切：非 daily 时清空显示
  const visibleDescs = scope === "daily" ? descs : [];
  const visibleSummaries = scope === "daily" ? summaries : [];

  return (
    <div className={styles.wrap}>
      {/* —— 顶部主行：报告范围 → 日期 → (右端) 重新生成 ——
          按"主操作"原则放在最显眼的第一行；参数 / 删除 / 导出走下方次行避免抢视线。 */}
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

        {/* 图/段下拉：跟 scope / 日期导航在同行，作为日报参数的一部分。
            "无限制" = 100000；跑总结时打包进 overrides 传给后端，本次生效不留痕。 */}
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

        {/* start / stop 放主行最右端：CSS margin-left:auto 推到行尾，"主操作"按钮放第一行
            最显眼位置。按钮文案缩短为"重新生成"避免在 960px 窗口里换行，旁边 Info icon hover 看完整说明。 */}
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
          <>
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
            <span
              className={styles.infoIconWrap}
              title={t("aiSummary.debug.actions.startInfo")}
              tabIndex={0}
              aria-label={t("aiSummary.debug.actions.startInfoAria")}
            >
              <Info size={13} strokeWidth={1.85} />
            </span>
          </>
        )}
      </div>


      {/* —— 引擎状态条 —— */}
      <EngineBar engine={engine} />

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

      {/* —— 逐图描述：包到 Section box，跟其他卡片视觉一致。
          headerAction：「仅生成图片描述」(只跑 step 1) + 「删除」(只清当天图片描述)。 */}
      <Section
        title={t("aiSummary.debug.perImage.title")}
        icon={ImageIcon}
        headerAction={
          <>
            <button
              type="button"
              className={styles.startBtn}
              onClick={() => void onStart("step1")}
              disabled={generating || !hasModel || scope !== "daily"}
              title={
                scope !== "daily"
                  ? t("aiSummary.debug.actions.startTooltipPending", { type: scopeName(scope) })
                  : hasModel
                    ? t("aiSummary.debug.actions.describeOnlyInfo")
                    : t("aiSummary.debug.actions.startTooltipNoModel")
              }
            >
              <Play size={13} strokeWidth={2} />
              {t("aiSummary.debug.actions.describeOnly")}
            </button>
            <button
              type="button"
              className={styles.deleteBtn}
              onClick={async () => {
                if (generating) return;
                if (
                  !confirm(
                    t("aiSummary.debug.actions.clearDescriptionsConfirm", { date }),
                  )
                )
                  return;
                try {
                  await api.clearDayImageDescriptions(date, "debug");
                  setDescs([]);
                } catch (e) {
                  setTopError(typeof e === "string" ? e : String(e));
                }
              }}
              disabled={generating || descs.length === 0}
              title={t("aiSummary.debug.actions.clearDescriptionsTooltip")}
            >
              <Trash2 size={13} strokeWidth={2} />
              {t("aiSummary.debug.actions.clearDescriptions")}
            </button>
            <button
              type="button"
              className={styles.exportBtn}
              onClick={onExportDescriptionsMd}
              disabled={descs.length === 0}
              title={
                descs.length === 0
                  ? t("aiSummary.debug.actions.exportDescriptionsMdEmptyTooltip")
                  : t("aiSummary.debug.actions.exportDescriptionsMdTooltip")
              }
            >
              <Download size={13} strokeWidth={2} />
              {t("aiSummary.debug.actions.exportDescriptionsMd")}
            </button>
          </>
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
                segment={segments[d.segmentIdx]}
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
                        ...(debugDescribeBatchSize != null
                          ? { describeBatchSize: debugDescribeBatchSize }
                          : {}),
                        ...(debugDescribeParallelSlots > 1
                          ? { describeParallelSlots: debugDescribeParallelSlots }
                          : {}),
                        ...(debugDescribeCtxSize != null
                          ? { describeCtxSize: debugDescribeCtxSize }
                          : {}),
                        ...(debugSummaryBatchSize != null
                          ? { summaryBatchSize: debugSummaryBatchSize }
                          : {}),
                        ...(debugSummaryParallelSlots > 1
                          ? { summaryParallelSlots: debugSummaryParallelSlots }
                          : {}),
                        ...(debugSummaryCtxSize != null
                          ? { summaryCtxSize: debugSummaryCtxSize }
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


      {/* —— 段总结结果：用 Section 跟「逐图描述」视觉对齐，常开 + 滚动。
          headerAction：「仅生成段总结」(跳过 step 1 用 DB 已存描述跑 step 2) + 「删除」(只清段总结)。 */}
      <Section
        title={t("aiSummary.debug.segments.title")}
        icon={Newspaper}
        headerAction={
          <>
            <button
              type="button"
              className={styles.startBtn}
              onClick={() => void onStart("step2")}
              disabled={generating || !hasModel || scope !== "daily"}
              title={
                scope !== "daily"
                  ? t("aiSummary.debug.actions.startTooltipPending", { type: scopeName(scope) })
                  : hasModel
                    ? t("aiSummary.debug.actions.summarizeOnlyInfo")
                    : t("aiSummary.debug.actions.startTooltipNoModel")
              }
            >
              <Play size={13} strokeWidth={2} />
              {t("aiSummary.debug.actions.summarizeOnly")}
            </button>
            <button
              type="button"
              className={styles.deleteBtn}
              onClick={async () => {
                if (generating) return;
                if (
                  !confirm(
                    t("aiSummary.debug.actions.clearSummariesConfirm", { date }),
                  )
                )
                  return;
                try {
                  await api.clearDaySegmentSummaries(date, "debug");
                  setSummaries([]);
                } catch (e) {
                  setTopError(typeof e === "string" ? e : String(e));
                }
              }}
              disabled={generating || summaries.length === 0}
              title={t("aiSummary.debug.actions.clearSummariesTooltip")}
            >
              <Trash2 size={13} strokeWidth={2} />
              {t("aiSummary.debug.actions.clearSummaries")}
            </button>
            <button
              type="button"
              className={styles.exportBtn}
              onClick={onExportSummariesMd}
              disabled={summaries.length === 0}
              title={
                summaries.length === 0
                  ? t("aiSummary.debug.actions.exportSummariesMdEmptyTooltip")
                  : t("aiSummary.debug.actions.exportSummariesMdTooltip")
              }
            >
              <Download size={13} strokeWidth={2} />
              {t("aiSummary.debug.actions.exportSummariesMd")}
            </button>
          </>
        }
      >
        {visibleSummaries.length === 0 ? (
          <div className={styles.summaryEmpty}>
            {t("aiSummary.debug.segments.empty")}
          </div>
        ) : (
          <div className={styles.panelOpen}>
            {visibleSummaries.map((s) => {
              // 段 chip 背景色：跟 DescItem / SegmentList / DailyTab 走同一份 fallback——
              // 配过 hex 用配置色，没配则按段中点色温渐变（早亮晚暗）。
              const seg = segments[s.segmentIdx];
              const { background: chipBg, isLight } = seg
                ? resolveSegmentChip(seg)
                : { background: "#cbd5e1", isLight: true };
              const chipColor = isLight ? "#3a3f55" : "#fff";
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
  segment,
  onRetry,
  retryDisabled,
  onOpenError,
}: {
  row: ImageDescriptionRow;
  segmentLabel?: string;
  segment?: AiSegment;
  onRetry: () => void;
  retryDisabled: boolean;
  /** 打开图片失败时上报错误给父组件展示（顶部 errorBar） */
  onOpenError: (msg: string) => void;
}) {
  const { t } = useTranslation();
  const fileName = row.screenshotPath.split(/[\\/]/).pop() ?? row.screenshotPath;
  // 截图时间（HH:MM）：直接从文件名解析（capture 写入约定 HHMMSS_NNN.jpg）
  const captureTime = extractScreenshotTime(row.screenshotPath);
  // chip 颜色：跟设置页 SegmentList / DailyTab 走同一份 fallback——配过 hex 用配置色，
  // 没配则按段中点的色温自动渐变。settings 还没加载 (segment === undefined) 时退回中性灰。
  const { background: chipBg, isLight } = segment
    ? resolveSegmentChip(segment)
    : { background: "#cbd5e1", isLight: true };
  const chipColor = isLight ? "#3a3f55" : "#fff";

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
        <span
          className={styles.descTime}
          title={t("aiSummary.debug.perImage.captureTimeTooltip", { time: captureTime })}
        >
          <Clock size={11} strokeWidth={2.2} />
          {captureTime}
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
                logWarn("debug.openPathFallback", e);
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

