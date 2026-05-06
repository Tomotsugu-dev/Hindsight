import { useEffect, useMemo, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { useMouseGlow } from "../../../hooks/useMouseGlow";
import {
  AlertTriangle,
  Check,
  ChevronLeft,
  ChevronRight,
  Clock,
  Download,
  Loader2,
  Play,
  RefreshCcw,
  RotateCcw,
  Square,
  Trash2,
} from "lucide-react";
import {
  api,
  SUMMARY_PROGRESS_EVENT,
  type AiSegment,
  type SegmentSummaryRow,
  type SummaryProgress,
} from "../../../api/hindsight";
import { useSettings } from "../../../state/settings";
import styles from "./DailyTab.module.css";

/** 段卡片在 UI 里的展示状态——由 ai_summaries 行 + 当前 in-flight 进度推导。 */
type CardState =
  | { kind: "empty" } // 还没生成 / 还没轮到
  | { kind: "running"; imagesTotal: number | null } // 段开跑中
  | { kind: "ok"; row: SegmentSummaryRow }
  | { kind: "skipped"; row: SegmentSummaryRow }
  | { kind: "error"; row: SegmentSummaryRow };

/** 把 dayOffset (0=今天 / -1=昨天 / ...) 转 "YYYY-MM-DD" 本地日期字符串。 */
function offsetToDateStr(dayOffset: number): string {
  const d = new Date();
  d.setDate(d.getDate() + dayOffset);
  const y = d.getFullYear();
  const m = String(d.getMonth() + 1).padStart(2, "0");
  const day = String(d.getDate()).padStart(2, "0");
  return `${y}-${m}-${day}`;
}

/** 把 dayOffset 转人话标签——0/-1 走"今天/昨天"，其它走相对日期。 */
function offsetLabel(dayOffset: number): string {
  if (dayOffset === 0) return "今天";
  if (dayOffset === -1) return "昨天";
  return offsetToDateStr(dayOffset);
}

/**
 * 日报 tab：当前一天的活动按段汇总 + 渐次出文。
 *
 * 进度通过 listen `ai://summary-progress` 流式更新；状态机：
 *   empty → running → (ok | skipped | error)
 */
export default function DailyTab() {
  const { settings } = useSettings();
  const [dayOffset, setDayOffset] = useState(0);
  const [rows, setRows] = useState<Map<number, SegmentSummaryRow>>(new Map());
  const [runningIdx, setRunningIdx] = useState<number | null>(null);
  const [runningImages, setRunningImages] = useState<number | null>(null);
  const [generating, setGenerating] = useState(false);
  const [enginePhase, setEnginePhase] = useState<string | null>(null);
  const [topError, setTopError] = useState<string | null>(null);
  // 导出 / 删除等操作完成后短暂显示的成功提示，3s 后自清
  const [topNotice, setTopNotice] = useState<string | null>(null);

  // 鼠标接近发光特效：Today 页同款，让日期导航更"活"
  const { ref: prevBtnRef } = useMouseGlow<HTMLButtonElement>();
  const { ref: pillRef } = useMouseGlow<HTMLButtonElement>();
  const { ref: nextBtnRef } = useMouseGlow<HTMLButtonElement>();

  const date = useMemo(() => offsetToDateStr(dayOffset), [dayOffset]);
  const segments = settings?.ai.segments ?? [];
  const activeMain = settings?.ai.activeMain ?? "";
  const hasModel = activeMain.trim().length > 0;

  // 进页 / 切日期：拉一次落库的总结，重置 in-flight 状态
  useEffect(() => {
    let cancelled = false;
    setRows(new Map());
    setRunningIdx(null);
    setRunningImages(null);
    setEnginePhase(null);
    setTopError(null);
    api
      .getDaySummary(date)
      .then((list) => {
        if (cancelled) return;
        const m = new Map<number, SegmentSummaryRow>();
        list.forEach((r) => m.set(r.segmentIdx, r));
        setRows(m);
      })
      .catch((e) => {
        if (!cancelled) console.error("getDaySummary 失败:", e);
      });
    return () => {
      cancelled = true;
    };
  }, [date]);

  // 用 ref 让 listen 闭包读到最新值（避免重新挂监听）
  const segmentsRef = useRef<AiSegment[]>(segments);
  segmentsRef.current = segments;
  const activeMainRef = useRef(activeMain);
  activeMainRef.current = activeMain;
  const dateRef = useRef(date);
  dateRef.current = date;

  useEffect(() => {
    const p = listen<SummaryProgress>(SUMMARY_PROGRESS_EVENT, (ev) => {
      const ev_ = ev.payload;
      // 只接 daily source 的事件——避免调试 tab 跑时这边也跟着刷
      if (ev_.source !== "daily") return;
      // 不是当前显示日期的事件忽略（用户切日期后旧任务还可能在跑）
      if (ev_.date !== dateRef.current) return;

      switch (ev_.phase) {
        case "engine_starting":
          setEnginePhase(ev_.message ?? "加载模型中…");
          break;
        case "segment_started":
          setEnginePhase(null);
          setRunningIdx(ev_.segmentIdx);
          setRunningImages(ev_.imagesTotal);
          break;
        case "segment_done": {
          if (ev_.segmentIdx == null || !ev_.status) break;
          const seg = segmentsRef.current[ev_.segmentIdx];
          if (!seg) break;
          const row: SegmentSummaryRow = {
            source: "daily",
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
          setRows((prev) => {
            const next = new Map(prev);
            next.set(ev_.segmentIdx!, row);
            return next;
          });
          setRunningIdx(null);
          setRunningImages(null);
          break;
        }
        case "all_done":
        case "cancelled":
          setRunningIdx(null);
          setRunningImages(null);
          setEnginePhase(null);
          setGenerating(false);
          break;
        case "error":
          setRunningIdx(null);
          setRunningImages(null);
          setEnginePhase(null);
          setGenerating(false);
          setTopError(ev_.message ?? "总结失败");
          break;
      }
    });
    return () => {
      void p.then((unlisten) => unlisten());
    };
  }, []);

  const onGenerate = async (forceRefresh: boolean) => {
    if (!hasModel) {
      setTopError("请先到 AI 设置 → 模型 选一个 vision 模型");
      return;
    }
    setGenerating(true);
    setTopError(null);
    try {
      await api.generateDaySummary(date, forceRefresh, null);
    } catch (e) {
      const msg = typeof e === "string" ? e : String(e);
      setTopError(msg);
      setGenerating(false);
    }
  };

  const onCancel = async () => {
    try {
      await api.cancelDaySummary();
    } catch (e) {
      console.warn("cancel 失败:", e);
    }
  };

  const onRetrySegment = async (segmentIdx: number) => {
    if (!hasModel) {
      setTopError("请先到 AI 设置 → 模型 选一个 vision 模型");
      return;
    }
    if (generating) return;
    setGenerating(true);
    setTopError(null);
    try {
      await api.retrySummarySegment(date, segmentIdx, null);
    } catch (e) {
      const msg = typeof e === "string" ? e : String(e);
      setTopError(msg);
    } finally {
      setGenerating(false);
    }
  };

  const allDone = segments.length > 0 && segments.every((_, i) => rows.has(i));
  const mainBtnLabel = generating
    ? "停止"
    : allDone
      ? "重新生成"
      : "开始总结";

  // 至少有一段已落库（不论 ok / skipped / error）就允许导出
  const hasAnyRow = rows.size > 0;

  /** 把当前日期的所有段总结拼成 Markdown 文件并下载。
   *  含 YAML frontmatter（date / generated_at / model / 状态汇总）+ 顶部总览
   *  + 每段状态徽章 + 段间 --- 分隔，方便贴 Notion / Obsidian / GitHub 渲染。 */
  const onExportMarkdown = () => {
    // 状态计数
    let okCount = 0;
    let skipCount = 0;
    let errCount = 0;
    let pendingCount = 0;
    let latestGeneratedAt: string | null = null;
    let modelName = "";

    segments.forEach((_, idx) => {
      const row = rows.get(idx);
      if (!row) {
        pendingCount += 1;
        return;
      }
      if (row.status === "ok") okCount += 1;
      else if (row.status === "skipped_no_screenshots") skipCount += 1;
      else errCount += 1;
      if (row.model) modelName = row.model;
      if (row.generatedAt && (!latestGeneratedAt || row.generatedAt > latestGeneratedAt)) {
        latestGeneratedAt = row.generatedAt;
      }
    });

    const dateLabel = offsetLabel(dayOffset);
    const summaryLine = [
      `共 ${segments.length} 段`,
      `${okCount} 已生成`,
      skipCount > 0 ? `${skipCount} 跳过` : null,
      errCount > 0 ? `${errCount} 失败` : null,
      pendingCount > 0 ? `${pendingCount} 未生成` : null,
    ]
      .filter(Boolean)
      .join(" · ");

    // YAML frontmatter
    const lines: string[] = ["---"];
    lines.push(`title: ${dateLabel}日报`);
    lines.push(`date: ${date}`);
    if (latestGeneratedAt) lines.push(`generated_at: ${latestGeneratedAt}`);
    if (modelName) lines.push(`model: ${modelName}`);
    lines.push(`segments: ${segments.length}`);
    lines.push(`status: ${okCount} ok / ${skipCount} skipped / ${errCount} error / ${pendingCount} pending`);
    lines.push("---", "");

    // 标题 + 总览
    lines.push(`# ${dateLabel}日报 · ${date}`, "");
    lines.push(`> 由 Hindsight AI 生成 · ${summaryLine}`, "");

    // 每段
    segments.forEach((seg, idx) => {
      const row = rows.get(idx);
      const range = `${String(seg.startHour).padStart(2, "0")}:00 – ${String(seg.endHour).padStart(2, "0")}:00`;
      lines.push(`## ${seg.label} · ${range}`, "");

      if (!row) {
        lines.push("⏳ **未生成**", "");
      } else if (row.status === "ok") {
        // 成功段直接放内容，不写"已生成"占位（frontmatter 已有状态汇总）
        lines.push(row.content?.trim() || "_（空）_", "");
      } else if (row.status === "skipped_no_screenshots") {
        lines.push("⚪ **跳过** — 这段时间没有截图", "");
      } else {
        lines.push("❌ **失败**", "");
        lines.push(`> ${(row.error || "未知错误").replace(/\n/g, "\n> ")}`, "");
      }

      // 段间分隔（最后一段不加）
      if (idx < segments.length - 1) {
        lines.push("---", "");
      }
    });

    const md = lines.join("\n");
    const blob = new Blob([md], { type: "text/markdown;charset=utf-8" });
    const url = URL.createObjectURL(blob);
    const filename = `hindsight-daily-${date}.md`;
    const a = document.createElement("a");
    a.href = url;
    a.download = filename;
    a.click();
    URL.revokeObjectURL(url);
    // 提示用户文件位置（浏览器下载，落到系统 Downloads 目录）
    setTopNotice(`已导出 ${filename} 到「下载」文件夹`);
    setTimeout(() => setTopNotice(null), 3500);
  };

  return (
    <>
      <p className={styles.subtitle}>
        按时段汇总当日截图 + 应用使用，本地 vision 模型生成
      </p>

      <header className={styles.header}>
        {/* 日期导航：< [今天] >，参照 Today 页（accent 描边 + glow 跟手） */}
        <div className={styles.dateNav}>
          <button
            ref={prevBtnRef}
            type="button"
            className={`${styles.navBtn} glow`}
            onClick={() => setDayOffset((v) => v - 1)}
            disabled={generating}
            aria-label="前一天"
          >
            <ChevronLeft size={14} strokeWidth={1.75} />
          </button>
          <button
            ref={pillRef}
            type="button"
            className={`${styles.dayPill} ${dayOffset !== 0 ? styles.dayPillClickable : ""} glow`}
            onClick={() => setDayOffset(0)}
            disabled={generating || dayOffset === 0}
            title={dayOffset === 0 ? undefined : "回到今天"}
          >
            {offsetLabel(dayOffset)}
          </button>
          <button
            ref={nextBtnRef}
            type="button"
            className={`${styles.navBtn} glow`}
            onClick={() => setDayOffset((v) => v + 1)}
            disabled={generating || dayOffset >= 0}
            aria-label="后一天"
          >
            <ChevronRight size={14} strokeWidth={1.75} />
          </button>
        </div>

        {/* 主操作按钮：紧跟日期导航 */}
        {generating ? (
          <button
            type="button"
            className={styles.stopBtn}
            onClick={() => void onCancel()}
          >
            <Square size={14} strokeWidth={2} />
            {mainBtnLabel}
          </button>
        ) : (
          <button
            type="button"
            className={styles.startBtn}
            onClick={() => void onGenerate(allDone)}
            disabled={!hasModel}
            title={
              hasModel
                ? allDone
                  ? "重新生成会清掉本日所有段总结，重新跑一遍"
                  : "本地 vision 模型逐段分析；时长取决于硬件（GPU 数秒/段，CPU 数十秒/段）"
                : "请先到 AI 设置 → 模型 选一个模型"
            }
          >
            {allDone ? (
              <RefreshCcw size={14} strokeWidth={2} />
            ) : (
              <Play size={14} strokeWidth={2} />
            )}
            {mainBtnLabel}
          </button>
        )}

        {/* 删除当天 daily 数据 */}
        <button
          type="button"
          className={styles.deleteBtn}
          onClick={async () => {
            if (generating) return;
            if (
              !confirm(
                `删除 ${date} 当天日报数据？（段总结 + 逐图描述都清空，不影响调试 tab，操作不可撤销）`,
              )
            )
              return;
            try {
              await api.clearDaySummary(date, "daily");
              setRows(new Map());
              setTopError(null);
            } catch (e) {
              setTopError(typeof e === "string" ? e : String(e));
            }
          }}
          disabled={generating || !hasAnyRow}
          title={
            !hasAnyRow
              ? "当天还没有日报数据可删除"
              : "清空当天 ai_summaries（source=daily）+ ai_image_descriptions（不可撤销）"
          }
        >
          <Trash2 size={14} strokeWidth={2} />
          删除
        </button>

        {/* 导出 Markdown：推到 header 最右 */}
        <button
          type="button"
          className={styles.exportBtn}
          onClick={onExportMarkdown}
          disabled={generating || !hasAnyRow}
          title={
            !hasAnyRow
              ? "还没有任何段总结可导出"
              : generating
                ? "总结进行中，等完成后再导出"
                : "导出当前日期的所有段总结为 Markdown 文件"
          }
        >
          <Download size={14} strokeWidth={2} />
          导出为 Markdown
        </button>
      </header>

      {topError ? (
        <div className={styles.errorBar}>
          <AlertTriangle size={14} strokeWidth={2.2} />
          <span>{topError}</span>
        </div>
      ) : null}

      {topNotice ? (
        <div className={styles.successBar}>
          <Check size={14} strokeWidth={2.4} />
          <span>{topNotice}</span>
        </div>
      ) : null}

      {enginePhase ? (
        <div className={styles.engineHint}>
          <Loader2 size={14} className={styles.spin} />
          <span>{enginePhase}</span>
        </div>
      ) : null}

      <div className={styles.cardList}>
        {segments.map((seg, idx) => {
          const row = rows.get(idx);
          const isRunning = runningIdx === idx;
          const state: CardState = isRunning
            ? { kind: "running", imagesTotal: runningImages }
            : !row
              ? { kind: "empty" }
              : row.status === "ok"
                ? { kind: "ok", row }
                : row.status === "skipped_no_screenshots"
                  ? { kind: "skipped", row }
                  : { kind: "error", row };

          return (
            <SegmentCard
              key={idx}
              seg={seg}
              state={state}
              onRetry={() => void onRetrySegment(idx)}
              retryDisabled={generating}
            />
          );
        })}
      </div>
    </>
  );
}

interface SegmentCardProps {
  seg: AiSegment;
  state: CardState;
  onRetry: () => void;
  retryDisabled: boolean;
}

function SegmentCard({ seg, state, onRetry, retryDisabled }: SegmentCardProps) {
  // chip 颜色：用户配过 hex 就用，否则浅灰
  const chipColor = seg.color || "#cbd5e1";
  return (
    <div className={styles.card}>
      <div className={styles.cardHead}>
        <span className={styles.chip} style={{ background: chipColor }}>
          {seg.label}
        </span>
        <span className={styles.timeRange}>
          <Clock size={12} strokeWidth={2.2} />
          {String(seg.startHour).padStart(2, "0")}:00 –{" "}
          {String(seg.endHour).padStart(2, "0")}:00
        </span>
        <CardStatusBadge state={state} />
        {state.kind === "error" || state.kind === "ok" ? (
          <button
            type="button"
            className={styles.retryBtn}
            onClick={onRetry}
            disabled={retryDisabled}
            title={retryDisabled ? "整轮总结进行中，等完成或停止后再重试" : "重新生成这一段"}
          >
            <RotateCcw size={12} strokeWidth={2.2} />
            重试
          </button>
        ) : null}
      </div>
      <CardBody state={state} />
    </div>
  );
}

function CardStatusBadge({ state }: { state: CardState }) {
  switch (state.kind) {
    case "empty":
      return <span className={`${styles.statusBadge} ${styles.statusEmpty}`}>未生成</span>;
    case "running":
      return (
        <span className={`${styles.statusBadge} ${styles.statusRunning}`}>
          <Loader2 size={11} className={styles.spin} />
          {state.imagesTotal != null && state.imagesTotal > 0
            ? `分析中 · ${state.imagesTotal} 张`
            : "分析中…"}
        </span>
      );
    case "ok":
      return <span className={`${styles.statusBadge} ${styles.statusOk}`}>已生成</span>;
    case "skipped":
      return <span className={`${styles.statusBadge} ${styles.statusSkipped}`}>无截图</span>;
    case "error":
      return <span className={`${styles.statusBadge} ${styles.statusError}`}>失败</span>;
  }
}

function CardBody({ state }: { state: CardState }) {
  switch (state.kind) {
    case "empty":
      return (
        <div className={styles.bodyMuted}>
          点上方「开始总结」生成本段内容。
        </div>
      );
    case "running":
      return (
        <div className={styles.bodyMuted}>
          {state.imagesTotal != null && state.imagesTotal > 0
            ? `正在让模型看完 ${state.imagesTotal} 张截图…`
            : "正在分析…"}
        </div>
      );
    case "ok":
      return <div className={styles.bodyText}>{state.row.content}</div>;
    case "skipped":
      return (
        <div className={styles.bodyMuted}>
          这段时间没有截图（可能没用电脑 / 全在隐私名单 / 全在排除分类）。
        </div>
      );
    case "error":
      return (
        <div className={styles.bodyError}>
          <strong>失败：</strong>
          {state.row.error || "未知错误"}
        </div>
      );
  }
}
