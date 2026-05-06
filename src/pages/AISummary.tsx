import { useEffect, useMemo, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import {
  AlertTriangle,
  ChevronLeft,
  ChevronRight,
  Clock,
  Loader2,
  Play,
  RefreshCcw,
  RotateCcw,
  Square,
} from "lucide-react";
import {
  api,
  SUMMARY_PROGRESS_EVENT,
  type AiSegment,
  type SegmentSummaryRow,
  type SummaryProgress,
} from "../api/hindsight";
import { useSettings } from "../state/settings";
import styles from "./AISummary.module.css";

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

export default function AISummary() {
  const { settings } = useSettings();
  const [dayOffset, setDayOffset] = useState(0);
  /** 服务端返回的当天落库行；map<segmentIdx, row> 方便覆盖更新 */
  const [rows, setRows] = useState<Map<number, SegmentSummaryRow>>(new Map());
  /** 当前段下标——in-flight 时该段卡显示 spinner */
  const [runningIdx, setRunningIdx] = useState<number | null>(null);
  /** 该段的图片总数提示 */
  const [runningImages, setRunningImages] = useState<number | null>(null);
  /** 整轮 generate 在飞 → 顶部主按钮变 "停止" + 锁日期切换 */
  const [generating, setGenerating] = useState(false);
  /** 引擎冷启动阶段提示 */
  const [enginePhase, setEnginePhase] = useState<string | null>(null);
  /** 顶层失败（如未选模型）的 toast */
  const [topError, setTopError] = useState<string | null>(null);

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

  // listen 全局进度事件——按 date 过滤，避免跨日期串场
  const dateRef = useRef(date);
  dateRef.current = date;
  useEffect(() => {
    const p = listen<SummaryProgress>(SUMMARY_PROGRESS_EVENT, (ev) => {
      const ev_ = ev.payload;
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
          // 用 progress payload 直接构造一行 row 覆盖到 map（避免再多调 getDaySummary）
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
          setRunningIdx(null);
          setRunningImages(null);
          setEnginePhase(null);
          setGenerating(false);
          break;
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
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []); // 一次性挂；通过 ref 拿最新 segments / activeMain

  // 用 ref 让 listen 闭包读到最新值（避免重新挂监听）
  const segmentsRef = useRef<AiSegment[]>(segments);
  segmentsRef.current = segments;
  const activeMainRef = useRef(activeMain);
  activeMainRef.current = activeMain;

  const onGenerate = async (forceRefresh: boolean) => {
    if (!hasModel) {
      setTopError("请先到 AI 设置 → 模型 选一个 vision 模型");
      return;
    }
    setGenerating(true);
    setTopError(null);
    try {
      await api.generateDaySummary(date, forceRefresh, null);
      // 命令 resolve = 全部段完成；事件流里 all_done 会把 generating 置 false。
      // 这里不动 generating，避免抢在 all_done 事件之前。
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
    if (generating) return; // 整轮在跑就不让单段重试
    setGenerating(true);
    setTopError(null);
    try {
      await api.retrySummarySegment(date, segmentIdx, null);
    } catch (e) {
      const msg = typeof e === "string" ? e : String(e);
      setTopError(msg);
    } finally {
      // 单段也走 segment_done → 我们手动收 generating
      // 但 retry 没有 all_done 事件，命令 resolve 时直接收尾
      setGenerating(false);
    }
  };

  const allDone = segments.length > 0 && segments.every((_, i) => rows.has(i));
  const mainBtnLabel = generating
    ? "停止"
    : allDone
      ? "重新生成"
      : "开始总结";

  return (
    <div className={styles.page}>
      <header className={styles.header}>
        <div className={styles.headLeft}>
          <h1 className={styles.title}>AI 总结</h1>
          <span className={styles.subtitle}>
            按时段汇总当日截图 + 应用使用，本地 vision 模型生成
          </span>
        </div>

        <div className={styles.headRight}>
          {/* 日期导航：< [今天] > */}
          <div className={styles.dateNav}>
            <button
              type="button"
              className={styles.navBtn}
              onClick={() => setDayOffset((v) => v - 1)}
              disabled={generating}
              aria-label="前一天"
            >
              <ChevronLeft size={16} strokeWidth={2.2} />
            </button>
            <button
              type="button"
              className={styles.dayPill}
              onClick={() => setDayOffset(0)}
              disabled={generating || dayOffset === 0}
              title="回到今天"
            >
              {offsetLabel(dayOffset)}
            </button>
            <button
              type="button"
              className={styles.navBtn}
              onClick={() => setDayOffset((v) => v + 1)}
              disabled={generating || dayOffset >= 0}
              aria-label="后一天"
            >
              <ChevronRight size={16} strokeWidth={2.2} />
            </button>
          </div>

          {/* 主操作按钮 */}
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
        </div>
      </header>

      {/* 顶部错误条 */}
      {topError ? (
        <div className={styles.errorBar}>
          <AlertTriangle size={14} strokeWidth={2.2} />
          <span>{topError}</span>
        </div>
      ) : null}

      {/* 引擎冷启动状态 */}
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
    </div>
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

