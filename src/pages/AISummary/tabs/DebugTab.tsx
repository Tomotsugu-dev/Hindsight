import { useEffect, useMemo, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import {
  AlertTriangle,
  ChevronLeft,
  ChevronRight,
  Download,
  Loader2,
  Play,
  RotateCcw,
  Square,
} from "lucide-react";
import {
  api,
  SUMMARY_PROGRESS_EVENT,
  type EngineStatus,
  type ImageDescriptionRow,
  type SegmentSummaryRow,
  type SummaryProgress,
} from "../../../api/hindsight";
import { useSettings } from "../../../state/settings";
import { SimplePicker } from "../../../components/SimplePicker/SimplePicker";
import styles from "./DebugTab.module.css";

/** 事件流 log 单条。 */
interface LogEntry {
  ts: string; // HH:MM:SS.mmm
  phase: SummaryProgress["phase"];
  body: string;
}

/** 调试 tab 顶部的"调什么"——目前只有日报有真后端，周报 / 月报先占位。 */
type DebugScope = "daily" | "weekly" | "monthly";

const SCOPE_OPTIONS: Array<{ value: DebugScope; label: string }> = [
  { value: "daily", label: "日报" },
  { value: "weekly", label: "周报" },
  { value: "monthly", label: "月报" },
];

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
function offsetLabel(scope: DebugScope, offset: number): string {
  if (scope === "daily") {
    if (offset === 0) return "今天";
    if (offset === -1) return "昨天";
    return anchorDateStr("daily", offset);
  }
  if (scope === "weekly") {
    if (offset === 0) return "本周";
    if (offset === -1) return "上周";
    if (offset === -2) return "上上周";
    return `${-offset} 周前`;
  }
  // monthly
  if (offset === 0) return "本月";
  if (offset === -1) return "上月";
  return `${-offset} 月前`;
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
  const { settings } = useSettings();
  const segments = settings?.ai.segments ?? [];
  const activeMain = settings?.ai.activeMain ?? "";
  const hasModel = activeMain.trim().length > 0;

  const [dayOffset, setDayOffset] = useState(0);
  /** 顶部"调什么"——日报 / 周报 / 月报；后两个先占位等后端实现 */
  const [scope, setScope] = useState<DebugScope>("daily");
  const [generating, setGenerating] = useState(false);
  const [enginePhase, setEnginePhase] = useState<string | null>(null);
  const [topError, setTopError] = useState<string | null>(null);

  const [engine, setEngine] = useState<EngineStatus | null>(null);
  const [descs, setDescs] = useState<ImageDescriptionRow[]>([]);
  const [summaries, setSummaries] = useState<SegmentSummaryRow[]>([]);
  const [logs, setLogs] = useState<LogEntry[]>([]);

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
      api.getDayImageDescriptions(date).catch(() => [] as ImageDescriptionRow[]),
      api.getDaySummary(date).catch(() => [] as SegmentSummaryRow[]),
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
          setEnginePhase(ev_.message ?? "加载模型中…");
          break;
        case "segment_started":
          setEnginePhase(null);
          break;
        case "image_described": {
          // 实时往描述列表插一条 / 更新已有项
          if (ev_.segmentIdx == null || ev_.imageIndex == null) break;
          const row: ImageDescriptionRow = {
            localDate: ev_.date,
            segmentIdx: ev_.segmentIdx,
            imageIndex: ev_.imageIndex,
            screenshotPath: ev_.imagePath ?? "",
            description: ev_.imageDescription ?? "",
            model: activeMainRef.current,
            generatedAt: new Date().toISOString(),
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
          setTopError(ev_.message ?? "调试运行失败");
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

  const onStart = async () => {
    if (scope !== "daily") {
      setTopError(`${scope === "weekly" ? "周报" : "月报"}调试待后端实现`);
      return;
    }
    if (!hasModel) {
      setTopError("请先到 AI 设置 → 模型 选一个 vision 模型");
      return;
    }
    setGenerating(true);
    setTopError(null);
    try {
      // 调试模式 = force_refresh，清掉旧的重新跑一遍看完整流程
      await api.generateDaySummary(date, true, null);
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
            type="button"
            className={styles.navBtn}
            onClick={() => setDayOffset((v) => v - 1)}
            disabled={generating}
            aria-label={
              scope === "daily" ? "前一天" : scope === "weekly" ? "上一周" : "上一月"
            }
          >
            <ChevronLeft size={16} strokeWidth={2.2} />
          </button>
          <button
            type="button"
            className={styles.dayPill}
            onClick={() => setDayOffset(0)}
            disabled={generating || dayOffset === 0}
            title={
              scope === "daily" ? "回到今天" : scope === "weekly" ? "回到本周" : "回到本月"
            }
          >
            {offsetLabel(scope, dayOffset)}
          </button>
          <button
            type="button"
            className={styles.navBtn}
            onClick={() => setDayOffset((v) => v + 1)}
            disabled={generating || dayOffset >= 0}
            aria-label={
              scope === "daily" ? "后一天" : scope === "weekly" ? "下一周" : "下一月"
            }
          >
            <ChevronRight size={16} strokeWidth={2.2} />
          </button>
        </div>

        {generating ? (
          <button
            type="button"
            className={styles.stopBtn}
            onClick={() => void onStop()}
          >
            <Square size={14} strokeWidth={2} />
            停止
          </button>
        ) : (
          <button
            type="button"
            className={styles.startBtn}
            onClick={() => void onStart()}
            disabled={!hasModel || scope !== "daily"}
            title={
              scope !== "daily"
                ? `${scope === "weekly" ? "周报" : "月报"}调试待后端实现`
                : hasModel
                  ? "调试模式：跑当天全部段（清空已有结果重跑），逐图描述实时显示"
                  : "请先到 AI 设置 → 模型 选一个模型"
            }
          >
            <Play size={14} strokeWidth={2} />
            开始
          </button>
        )}

        <button
          type="button"
          className={styles.exportBtn}
          onClick={onExport}
          disabled={
            descs.length === 0 && summaries.length === 0 && logs.length === 0
          }
          title="导出当前页所有调试数据为 JSON 文件"
        >
          <Download size={13} strokeWidth={2} />
          导出 JSON
        </button>
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
          {scope === "weekly" ? "周报" : "月报"}调试待后端实现。
          切回「日报」可以调试当前日报的两步生成流程（逐图描述 → 段总结）。
        </div>
      ) : null}

      {/* —— Prompt 实际文本预览（折叠 - 待后端 preview 命令上线接活） —— */}
      <div className={styles.panelWrap}>
        <span className={styles.panelLabel}>Prompt 实际文本预览</span>
        <div className={styles.panel}>
          <p className={styles.promptSubLabel}>Image describe（step 1 system / user）</p>
          <div className={styles.placeholder}>
            待后端 <code>preview_summary_prompts</code> 命令上线后，这里会显示按当前
            settings.ai.promptLanguage + user_brief 实际拼好的 system / user prompt 文本。
          </div>
          <p className={styles.promptSubLabel}>Segment summary（step 2 system / user）</p>
          <div className={styles.placeholder}>
            待后端命令上线。step 2 是纯文本调用，user prompt 包含本段所有 image_descriptions
            展开 + top apps 列表。
          </div>
        </div>
      </div>

      {/* —— 逐图描述列表 —— */}
      <div className={styles.descListWrap}>
        <div className={styles.descListLabel}>
          逐图描述
          {visibleDescs.length > 0 ? (
            <span style={{ color: "var(--text-faint)" }}>
              · {visibleDescs.length} 条
            </span>
          ) : null}
        </div>
        {visibleDescs.length === 0 ? (
          <div className={styles.descListEmpty}>
            还没有数据。点上方「开始」让模型对每张截图生成描述。
          </div>
        ) : (
          visibleDescs.map((d) => (
            <DescItem
              key={`${d.segmentIdx}-${d.imageIndex}`}
              row={d}
              segmentLabel={segments[d.segmentIdx]?.label}
            />
          ))
        )}
      </div>

      {/* —— 段总结 step 2 完整 user prompt（折叠 - 待后端 preview 命令） —— */}
      <div className={styles.panelWrap}>
        <span className={styles.panelLabel}>段总结 step 2 完整 user prompt</span>
        <div className={styles.panel}>
          <div className={styles.placeholder}>
            待后端 <code>preview_summary_prompts</code>{" "}
            命令上线后，这里展示当前段实际发出去的 step 2 user prompt
            （含描述列表逐条展开 + top apps + 时段元数据）。
          </div>
        </div>
      </div>

      {/* —— 段总结结果 —— */}
      <div className={styles.panelWrap}>
        <span className={styles.panelLabel}>段总结结果</span>
        <div className={styles.panel}>
          {visibleSummaries.length === 0 ? (
            <div className={styles.summaryEmpty}>还没有段总结结果。</div>
          ) : (
            visibleSummaries.map((s) => (
              <div key={s.segmentIdx} className={styles.summaryBox}>
                <span className={styles.summaryLabel}>
                  #{s.segmentIdx} {s.label}（{String(s.startHour).padStart(2, "0")}–
                  {String(s.endHour).padStart(2, "0")}）· {s.status}
                </span>
                <div className={styles.summaryText}>
                  {s.content ||
                    (s.status === "skipped_no_screenshots"
                      ? "(该段无截图)"
                      : s.error || "(空)")}
                </div>
              </div>
            ))
          )}
        </div>
      </div>

      {/* —— 实时事件流 —— */}
      <div className={styles.panelWrap}>
        <span className={styles.panelLabel}>事件流（rolling 200 条）</span>
        <div className={styles.panel}>
          <div className={styles.logBox}>
            {logs.length === 0 ? (
              <div className={styles.logEmpty}>暂无事件。点开始后这里会滚动出现。</div>
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
    </div>
  );
}

/** 引擎状态条：端口 / 模型 / ctx / 状态指示 dot。 */
function EngineBar({ engine }: { engine: EngineStatus | null }) {
  if (!engine) {
    return (
      <div className={styles.engineBar}>
        <span className={styles.engineDot} />
        <span>引擎状态加载中…</span>
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
    : "未安装";
  return (
    <div className={styles.engineBar}>
      <span className={`${styles.engineDot} ${dotClass}`} />
      <span>
        端口：
        <span className={styles.engineMetaStrong}>
          {rt.state === "running" && rt.port != null ? `:${rt.port}` : "—"}
        </span>
      </span>
      <span className={styles.engineSep}>·</span>
      <span>
        llama.cpp 版本：
        <span className={styles.engineMetaStrong}>{versionStr}</span>
      </span>
      <span className={styles.engineSep}>·</span>
      <span>
        加速：
        <span
          className={styles.engineMetaStrong}
          title={`binary 变体 ID: ${engine.platformId}`}
        >
          {humanAccelLabel(engine.platformId)}
        </span>
      </span>
      <span className={styles.engineSep}>·</span>
      <span>
        状态：
        <span className={styles.engineMetaStrong}>{rt.state}</span>
      </span>
      {rt.error ? (
        <>
          <span className={styles.engineSep}>·</span>
          <span style={{ color: "#dc2626" }}>错误：{rt.error}</span>
        </>
      ) : null}
    </div>
  );
}

/** 单条逐图描述项。耗时 / token / 单图重跑都是后端待补，UI 先留位置。 */
function DescItem({
  row,
  segmentLabel,
}: {
  row: ImageDescriptionRow;
  segmentLabel?: string;
}) {
  const fileName = row.screenshotPath.split(/[\\/]/).pop() ?? row.screenshotPath;
  return (
    <div className={styles.descItem}>
      <div className={styles.descMeta}>
        <span
          className={styles.descIndex}
          title={`段 idx=${row.segmentIdx} · 图 idx=${row.imageIndex}（抽帧后顺序）`}
        >
          {segmentLabel ?? `段${row.segmentIdx}`} · 第 {row.imageIndex + 1} 张
        </span>
        <span className={styles.descPath} title={row.screenshotPath}>
          {fileName}
        </span>
        <span
          className={styles.descStat}
          title="耗时 / prompt token / completion token —— 待后端 v20 + ChatUsage 落地后填实"
        >
          耗时 — · token —
        </span>
        <button
          type="button"
          className={styles.retryImg}
          disabled
          title="待后端 retry_single_image_description 命令上线后启用"
        >
          <RotateCcw size={11} strokeWidth={2.2} />
          重跑
        </button>
      </div>
      <div className={styles.descText}>{row.description || "(空)"}</div>
    </div>
  );
}
