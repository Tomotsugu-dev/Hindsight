import { useEffect, useMemo, useRef, useState, useSyncExternalStore } from "react";
import { useTranslation } from "react-i18next";
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
  type AiSegment,
  type SegmentSummaryRow,
} from "../../../api/hindsight";
import { useSettings } from "../../../state/settings";
import { resolveSegmentChip } from "../../../utils/segmentColor";
import {
  cancelDailyGenerate,
  clearTopError,
  getDailyRunningSnapshot,
  retryDailySegment,
  setTopError,
  startDailyGenerate,
  subscribeDailySummary,
  subscribeSegmentDone,
} from "../../../state/dailySummary";
import styles from "./DailyTab.module.css";

/** 段卡片在 UI 里的展示状态——由 ai_summaries 行 + 当前 in-flight 进度推导。 */
type CardState =
  | { kind: "empty" } // 还没生成 / 还没轮到
  | { kind: "running"; imagesTotal: number | null; imagesDone: number } // 段开跑中
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

/**
 * 日报 tab：当前一天的活动按段汇总 + 渐次出文。
 *
 * 进度通过 listen `ai://summary-progress` 流式更新；状态机：
 *   empty → running → (ok | skipped | error)
 */
export default function DailyTab() {
  const { t } = useTranslation();
  const { settings } = useSettings();
  const [dayOffset, setDayOffset] = useState(0);
  const [rows, setRows] = useState<Map<number, SegmentSummaryRow>>(new Map());
  // 在跑状态全部从 module-level store 读，组件 unmount 不丢；切走再回来按钮、
  // engine hint、当前段 spinner 都自动恢复。
  const runSnap = useSyncExternalStore(
    subscribeDailySummary,
    getDailyRunningSnapshot,
    getDailyRunningSnapshot,
  );
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

  // 把 dayOffset 转人话标签——0/-1 走"今天/昨天"，其它走相对日期。
  // 依赖 t，所以在组件里定义，跟随 i18n.language 自动重渲。
  const offsetLabel = (off: number): string => {
    if (off === 0) return t("aiSummary.daily.dateNav.today");
    if (off === -1) return t("aiSummary.daily.dateNav.yesterday");
    return offsetToDateStr(off);
  };

  // 进页 / 切日期：拉一次落库的总结。run 状态由 store 接管，不在这里重置——
  // 切走再回来还在跑的 daily run 应该让按钮保持"停止"，不能清掉。
  useEffect(() => {
    let cancelled = false;
    setRows(new Map());
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

  // 用 ref 让 segmentDone 回调读到最新值（避免每次 segments / activeMain 变都
  // 重订阅 listener；这两个值变化频率很低也不影响事件流处理逻辑）
  const segmentsRef = useRef<AiSegment[]>(segments);
  segmentsRef.current = segments;
  const activeMainRef = useRef(activeMain);
  activeMainRef.current = activeMain;
  const dateRef = useRef(date);
  dateRef.current = date;

  // 段完成事件：store 派发给当前 mount 的 DailyTab，让它把 row 落到 rows Map。
  // 只在事件 date 跟当前 view date 一致时更新——切日期后旧 run 仍可能在发事件。
  useEffect(() => {
    return subscribeSegmentDone((ev) => {
      if (ev.date !== dateRef.current) return;
      if (ev.segmentIdx == null || !ev.status) return;
      const seg = segmentsRef.current[ev.segmentIdx];
      if (!seg) return;
      const row: SegmentSummaryRow = {
        source: "daily",
        localDate: ev.date,
        segmentIdx: ev.segmentIdx,
        label: seg.label,
        startHour: seg.startHour,
        endHour: seg.endHour,
        content: ev.content ?? "",
        model: activeMainRef.current,
        status: ev.status,
        error: ev.message ?? null,
        generatedAt: new Date().toISOString(),
      };
      setRows((prev) => {
        const next = new Map(prev);
        next.set(ev.segmentIdx!, row);
        return next;
      });
    });
  }, []);

  const onGenerate = async (forceRefresh: boolean) => {
    if (!hasModel) {
      setTopError(t("aiSummary.daily.errors.noVisionModel"));
      return;
    }
    try {
      await startDailyGenerate(date, forceRefresh);
    } catch {
      // store 已经设过 topError；这里不再重复处理
    }
  };

  const onCancel = async () => {
    await cancelDailyGenerate();
  };

  const onRetrySegment = async (segmentIdx: number) => {
    if (!hasModel) {
      setTopError(t("aiSummary.daily.errors.noVisionModel"));
      return;
    }
    if (runSnap.generating) return;
    try {
      await retryDailySegment(date, segmentIdx);
    } catch {
      // store 已经设过 topError
    }
  };

  // 是否在跑 = store 标 generating + 跑的日期跟当前 view 日期一致；
  // 切到别的日期看时按钮回到"开始总结"，不该被另一天的 run 拖累
  const isRunningHere =
    runSnap.generating && runSnap.runningDate === date;
  const runningIdx = isRunningHere ? runSnap.runningIdx : null;
  const runningImages = isRunningHere ? runSnap.runningImages : null;
  const runningDone = isRunningHere ? runSnap.runningDone : 0;
  const enginePhase = isRunningHere ? runSnap.enginePhase : null;
  const topError = runSnap.topError;
  const generating = isRunningHere;

  const allDone = segments.length > 0 && segments.every((_, i) => rows.has(i));
  const mainBtnLabel = generating
    ? t("aiSummary.daily.actions.stop")
    : allDone
      ? t("aiSummary.daily.actions.regenerate")
      : t("aiSummary.daily.actions.start");

  // 至少有一段已落库（不论 ok / skipped / error）就允许导出
  const hasAnyRow = rows.size > 0;

  /** 把当前日期的所有段总结拼成 Markdown 文件并下载。
   *  含 YAML frontmatter（date / generated_at / model / 状态汇总）+ 顶部总览
   *  + 每段状态徽章 + 段间 --- 分隔，方便贴 Notion / Obsidian / GitHub 渲染。
   *  导出文本本身随当前 i18n 语言生成。 */
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
      t("aiSummary.daily.export.segmentsCount", { count: segments.length }),
      t("aiSummary.daily.export.okCount", { count: okCount }),
      skipCount > 0 ? t("aiSummary.daily.export.skipCount", { count: skipCount }) : null,
      errCount > 0 ? t("aiSummary.daily.export.errCount", { count: errCount }) : null,
      pendingCount > 0 ? t("aiSummary.daily.export.pendingCount", { count: pendingCount }) : null,
    ]
      .filter(Boolean)
      .join(t("aiSummary.daily.export.summaryJoin"));

    // YAML frontmatter（label 用语言无关的英文 key，便于程序解析；只翻译 value）
    const lines: string[] = ["---"];
    lines.push(`title: ${t("aiSummary.daily.export.frontmatterTitle", { dateLabel })}`);
    lines.push(`date: ${date}`);
    if (latestGeneratedAt) lines.push(`generated_at: ${latestGeneratedAt}`);
    if (modelName) lines.push(`model: ${modelName}`);
    lines.push(`segments: ${segments.length}`);
    lines.push(`status: ${okCount} ok / ${skipCount} skipped / ${errCount} error / ${pendingCount} pending`);
    lines.push("---", "");

    // 标题 + 总览
    lines.push(`# ${t("aiSummary.daily.export.heading", { dateLabel, date })}`, "");
    lines.push(`> ${t("aiSummary.daily.export.intro", { summary: summaryLine })}`, "");

    // 每段
    segments.forEach((seg, idx) => {
      const row = rows.get(idx);
      const range = `${String(seg.startHour).padStart(2, "0")}:00 – ${String(seg.endHour).padStart(2, "0")}:00`;
      lines.push(`## ${seg.label} · ${range}`, "");

      if (!row) {
        lines.push(t("aiSummary.daily.export.statePending"), "");
      } else if (row.status === "ok") {
        // 成功段直接放内容，不写"已生成"占位（frontmatter 已有状态汇总）
        lines.push(row.content?.trim() || t("aiSummary.daily.export.stateEmptyContent"), "");
      } else if (row.status === "skipped_no_screenshots") {
        lines.push(t("aiSummary.daily.export.stateSkipped"), "");
      } else {
        lines.push(t("aiSummary.daily.export.stateError"), "");
        lines.push(`> ${(row.error || t("aiSummary.daily.errors.unknown")).replace(/\n/g, "\n> ")}`, "");
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
    setTopNotice(t("aiSummary.daily.toast.exported", { filename }));
    setTimeout(() => setTopNotice(null), 3500);
  };

  return (
    <>
      <p className={styles.subtitle}>{t("aiSummary.daily.subtitle")}</p>

      <header className={styles.header}>
        {/* 日期导航：< [今天] >，参照 Today 页（accent 描边 + glow 跟手） */}
        <div className={styles.dateNav}>
          <button
            ref={prevBtnRef}
            type="button"
            className={`${styles.navBtn} glow`}
            onClick={() => setDayOffset((v) => v - 1)}
            disabled={generating}
            aria-label={t("aiSummary.daily.dateNav.prevAria")}
          >
            <ChevronLeft size={14} strokeWidth={1.75} />
          </button>
          <button
            ref={pillRef}
            type="button"
            className={`${styles.dayPill} ${dayOffset !== 0 ? styles.dayPillClickable : ""} glow`}
            onClick={() => setDayOffset(0)}
            disabled={generating || dayOffset === 0}
            title={dayOffset === 0 ? undefined : t("aiSummary.daily.dateNav.todayBack")}
          >
            {offsetLabel(dayOffset)}
          </button>
          <button
            ref={nextBtnRef}
            type="button"
            className={`${styles.navBtn} glow`}
            onClick={() => setDayOffset((v) => v + 1)}
            disabled={generating || dayOffset >= 0}
            aria-label={t("aiSummary.daily.dateNav.nextAria")}
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
                  ? t("aiSummary.daily.actions.regenerateTooltip")
                  : t("aiSummary.daily.actions.startTooltip")
                : t("aiSummary.daily.actions.noModelTooltip")
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
                t("aiSummary.daily.actions.deleteConfirm", { date }),
              )
            )
              return;
            try {
              await api.clearDaySummary(date, "daily");
              setRows(new Map());
              clearTopError();
            } catch (e) {
              setTopError(typeof e === "string" ? e : String(e));
            }
          }}
          disabled={generating || !hasAnyRow}
          title={
            !hasAnyRow
              ? t("aiSummary.daily.actions.deleteEmptyTooltip")
              : t("aiSummary.daily.actions.deleteTooltip")
          }
        >
          <Trash2 size={14} strokeWidth={2} />
          {t("aiSummary.daily.actions.delete")}
        </button>

        {/* 导出 Markdown：推到 header 最右 */}
        <button
          type="button"
          className={styles.exportBtn}
          onClick={onExportMarkdown}
          disabled={generating || !hasAnyRow}
          title={
            !hasAnyRow
              ? t("aiSummary.daily.actions.exportEmptyTooltip")
              : generating
                ? t("aiSummary.daily.actions.exportRunningTooltip")
                : t("aiSummary.daily.actions.exportTooltip")
          }
        >
          <Download size={14} strokeWidth={2} />
          {t("aiSummary.daily.actions.exportMarkdown")}
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
            ? { kind: "running", imagesTotal: runningImages, imagesDone: runningDone }
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
  const { t } = useTranslation();
  // chip 颜色：跟设置页 SegmentList 走同一份 fallback——配过 hex 用配置色，
  // 没配则按段中点的色温自动渐变（早亮晚暗），保证两边视觉一致。
  const { background: chipBg, isLight } = resolveSegmentChip(seg);
  const chipFg = isLight ? "#3a3f55" : "#fff";
  return (
    <div className={styles.card}>
      <div className={styles.cardHead}>
        <span
          className={styles.chip}
          style={{ background: chipBg, color: chipFg }}
        >
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
            title={
              retryDisabled
                ? t("aiSummary.daily.actions.retryDisabledTooltip")
                : t("aiSummary.daily.actions.retryTooltip")
            }
          >
            <RotateCcw size={12} strokeWidth={2.2} />
            {t("aiSummary.daily.actions.retry")}
          </button>
        ) : null}
      </div>
      <CardBody state={state} />
    </div>
  );
}

function CardStatusBadge({ state }: { state: CardState }) {
  const { t } = useTranslation();
  switch (state.kind) {
    case "empty":
      return (
        <span className={`${styles.statusBadge} ${styles.statusEmpty}`}>
          {t("aiSummary.daily.card.badge.empty")}
        </span>
      );
    case "running":
      return (
        <span className={`${styles.statusBadge} ${styles.statusRunning}`}>
          <Loader2 size={11} className={styles.spin} />
          {state.imagesTotal != null && state.imagesTotal > 0
            ? t("aiSummary.daily.card.badge.analyzing", {
                done: state.imagesDone,
                total: state.imagesTotal,
              })
            : t("aiSummary.daily.card.badge.analyzingNoTotal")}
        </span>
      );
    case "ok":
      return (
        <span className={`${styles.statusBadge} ${styles.statusOk}`}>
          {t("aiSummary.daily.card.badge.ok")}
        </span>
      );
    case "skipped":
      return (
        <span className={`${styles.statusBadge} ${styles.statusSkipped}`}>
          {t("aiSummary.daily.card.badge.skipped")}
        </span>
      );
    case "error":
      return (
        <span className={`${styles.statusBadge} ${styles.statusError}`}>
          {t("aiSummary.daily.card.badge.error")}
        </span>
      );
  }
}

function CardBody({ state }: { state: CardState }) {
  const { t } = useTranslation();
  switch (state.kind) {
    case "empty":
      return (
        <div className={styles.bodyMuted}>
          {t("aiSummary.daily.card.body.empty")}
        </div>
      );
    case "running":
      return (
        <div className={styles.bodyMuted}>
          {state.imagesTotal != null && state.imagesTotal > 0
            ? t("aiSummary.daily.card.body.analyzing", {
                done: state.imagesDone,
                total: state.imagesTotal,
              })
            : t("aiSummary.daily.card.body.analyzingNoTotal")}
        </div>
      );
    case "ok":
      return <div className={styles.bodyText}>{state.row.content}</div>;
    case "skipped":
      return (
        <div className={styles.bodyMuted}>
          {t("aiSummary.daily.card.body.skipped")}
        </div>
      );
    case "error":
      return (
        <div className={styles.bodyError}>
          <strong>{t("aiSummary.daily.card.body.errorLabel")}</strong>
          {state.row.error || t("aiSummary.daily.errors.unknown")}
        </div>
      );
  }
}
