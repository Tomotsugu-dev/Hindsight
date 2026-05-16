import { useEffect, useMemo, useState, useSyncExternalStore } from "react";
import { useTranslation } from "react-i18next";
import { invoke } from "@tauri-apps/api/core";
import { save } from "@tauri-apps/plugin-dialog";
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
  Square,
  Trash2,
  X,
} from "lucide-react";
import { useMouseGlow } from "../../../hooks/useMouseGlow";
import { logError } from "../../../lib/logger";
import { useSettings } from "../../../state/settings";
import { ConfirmDialog } from "../../../components/ConfirmDialog/ConfirmDialog";
import {
  cancelWeeklyGenerate,
  clearTopError,
  fetchWeeklyRow,
  getWeeklyRunningSnapshot,
  setTopError,
  startWeeklyGenerate,
  subscribeWeeklyDone,
  subscribeWeeklySummary,
} from "../../../state/weeklySummary";
import {
  api,
  SUMMARY_CLOUD_SENTINEL,
  type SegmentSummaryRow,
  type WeekPrecheckDay,
  type WeekPrecheckResp,
} from "../../../api/hindsight";
import styles from "./WeeklyTab.module.css";

/** 把 weekOffset (0=本周 / -1=上周 / ...) 转成 [周一, 周日] 两个 "YYYY-MM-DD"。 */
function weekRangeFromOffset(weekOffset: number): { monday: string; sunday: string } {
  const today = new Date();
  // num_days_from_monday：getDay() 返 0=Sun..6=Sat；转成"距周一的天数"
  const dow = (today.getDay() + 6) % 7;
  const monday = new Date(today);
  monday.setDate(today.getDate() - dow + weekOffset * 7);
  const sunday = new Date(monday);
  sunday.setDate(monday.getDate() + 6);
  return { monday: toDateStr(monday), sunday: toDateStr(sunday) };
}

function toDateStr(d: Date): string {
  const y = d.getFullYear();
  const m = String(d.getMonth() + 1).padStart(2, "0");
  const day = String(d.getDate()).padStart(2, "0");
  return `${y}-${m}-${day}`;
}

/** 把 "YYYY-MM-DD" 转成 "MM-DD" 简写（节省周 pill 的横向空间）。 */
function toShortDate(s: string): string {
  return s.length === 10 ? s.slice(5) : s;
}

/** 卡片当前展示状态——由"是否在跑" + 落库 row 推导。 */
type CardState =
  | { kind: "empty" }
  | { kind: "running"; stage: "engine_starting" | "summarizing" }
  | { kind: "ok"; row: SegmentSummaryRow }
  | { kind: "error"; row: SegmentSummaryRow };

/**
 * 周报 tab：单步纯文本生成 + 单一卡片渲染。
 *
 * 跟 DailyTab 同款架构（顶部头 + 错误条 + 卡片）但是简化：
 * - 没有"段"列表，只渲染一个总结卡片
 * - 进度只看 stage（engine_starting / summarizing），无 N/M 张这种细粒度
 * - 点击「开始/重新生成」时先调 [`api.precheckWeekSummary`] 看本周日报覆盖情况：
 *   全有 → 直接生成；部分 / 整周缺 → 弹 ConfirmDialog 让用户选"继续生成"，
 *   确认后以 `allowMissingDays=true` 跑后端（缺日报的天用当日 top apps 顶替进 prompt）
 * - 失败原因常见两类：模型未配置 + "本周完全没数据"——后者由后端写 error 行，
 *   前端按错误展示
 */
export default function WeeklyTab() {
  const { t } = useTranslation();
  const { settings } = useSettings();
  const [weekOffset, setWeekOffset] = useState(0);
  const [row, setRow] = useState<SegmentSummaryRow | null>(null);
  const [topNotice, setTopNotice] = useState<string | null>(null);
  const [confirmingDelete, setConfirmingDelete] = useState(false);
  /** 点击生成后 precheck 检出缺失日，弹"是否继续"确认前的待办上下文。
   *  null = 无弹框；非 null = ConfirmDialog 打开中。
   *  forceRefresh 透传给确认后真正发起的 generate 调用，保证语义一致。 */
  const [pendingConfirm, setPendingConfirm] = useState<{
    kind: "partial" | "full";
    missingDays: WeekPrecheckDay[];
    activityOnlyCount: number;
    forceRefresh: boolean;
  } | null>(null);

  const { ref: prevBtnRef } = useMouseGlow<HTMLButtonElement>();
  const { ref: pillRef } = useMouseGlow<HTMLButtonElement>();
  const { ref: nextBtnRef } = useMouseGlow<HTMLButtonElement>();

  const { monday, sunday } = useMemo(() => weekRangeFromOffset(weekOffset), [weekOffset]);

  // hasModel：周报只走 step 2（纯文本），所以条件比 daily 宽松——本地 summary 模型
  // 或选定云端 二选一即可。describeMain 不参与判断（不过 vision）。
  const activeMain = settings?.ai.activeMain ?? "";
  const rawSummaryMain = settings?.ai.summaryMain ?? "";
  const externalEnabled = settings?.ai.externalEnabled ?? false;
  const cloudRoute = externalEnabled && rawSummaryMain === SUMMARY_CLOUD_SENTINEL;
  const localSummaryAvailable =
    (rawSummaryMain !== "" && rawSummaryMain !== SUMMARY_CLOUD_SENTINEL) ||
    activeMain !== "";
  const hasModel = cloudRoute || localSummaryAvailable;

  // 全局 store 订阅——切走再回来还在跑的 weekly run 仍能保持"停止"按钮
  const runSnap = useSyncExternalStore(
    subscribeWeeklySummary,
    getWeeklyRunningSnapshot,
    getWeeklyRunningSnapshot,
  );

  // 切周时清掉缺失日确认弹框——避免用户在 A 周点生成、弹框出现、切到 B 周后点
  // "继续生成"误用 A 周的缺失日列表跑 B 周的现象
  useEffect(() => {
    setPendingConfirm(null);
  }, [monday]);

  // 切周 / 进页：拉一次落库行
  useEffect(() => {
    let cancelled = false;
    setRow(null);
    fetchWeeklyRow(monday)
      .then((r) => {
        if (cancelled) return;
        setRow(r);
      })
      .catch((e) => {
        if (cancelled) return;
        logError("weekly.getSummary", e);
        setTopError(typeof e === "string" ? e : String(e));
      });
    return () => {
      cancelled = true;
    };
  }, [monday]);

  // segment_done 事件：store 派发，让组件把后端写好的 row 落到本地 state
  useEffect(() => {
    return subscribeWeeklyDone((ev) => {
      // 只有事件 date == 当前 view monday 才更新；切周后旧 run 仍可能发事件
      if (ev.date !== monday) return;
      if (!ev.status) return;
      // 后端 generate_week_summary 跑完会写好 row；这里立即重拉一次 DB 拿全量字段
      // （事件 payload 只带 content + status + message，缺 model / generatedAt 等）
      fetchWeeklyRow(monday)
        .then((r) => {
          if (r) setRow(r);
        })
        .catch((e) => logError("weekly.refetchAfterDone", e));
    });
  }, [monday]);

  const onGenerate = async (forceRefresh: boolean) => {
    if (!hasModel) {
      setTopError(t("aiSummary.weekly.errors.noSummaryModel"));
      return;
    }
    clearTopError();

    // precheck 先看缺日报情况——失败不阻塞，按"没问题"走兜底（后端严格模式仍会写
    // error 行，前端能看到提示）
    let precheck: WeekPrecheckResp;
    try {
      precheck = await api.precheckWeekSummary(monday);
    } catch (e) {
      logError("weekly.precheck", e);
      // 直接走严格生成；后端日报为空就 error 行兜底
      try {
        await startWeeklyGenerate(monday, forceRefresh);
      } catch {
        /* store 已设过 topError */
      }
      return;
    }

    const missingDays = precheck.days.filter((d) => !d.hasDaily);

    // 全有 → 直接生成
    if (missingDays.length === 0) {
      try {
        await startWeeklyGenerate(monday, forceRefresh);
      } catch {
        /* store 已设过 topError */
      }
      return;
    }

    // 整周缺 vs 部分缺：弹对应确认框
    setPendingConfirm({
      kind: precheck.daysWithDaily === 0 ? "full" : "partial",
      missingDays,
      activityOnlyCount: precheck.daysActivityOnly,
      forceRefresh,
    });
  };

  /** 用户在缺失日确认弹框点"继续生成"——以 allowMissingDays=true 跑后端。 */
  const onConfirmMissing = async () => {
    if (!pendingConfirm) return;
    const force = pendingConfirm.forceRefresh;
    setPendingConfirm(null);
    try {
      await startWeeklyGenerate(monday, force, true);
    } catch {
      /* store 已设过 topError */
    }
  };

  /** 把缺失日数组拼成 "MM-DD 周三, MM-DD 周四" 形式给 ConfirmDialog message。
   *  weekday 由后端按当前 prompt 语言生成；分隔符用 ASCII 逗号兼容三语。 */
  const formatMissingDates = (days: WeekPrecheckDay[]): string =>
    days.map((d) => `${toShortDate(d.date)} ${d.weekday}`).join(", ");

  const onCancel = async () => {
    await cancelWeeklyGenerate();
  };

  const isRunningHere = runSnap.generating && runSnap.runningWeek === monday;
  const stage = isRunningHere ? runSnap.stage : "idle";
  const enginePhase = isRunningHere ? runSnap.enginePhase : null;
  const topError = runSnap.topError;
  const generating = isRunningHere;

  const hasOk = row?.status === "ok";
  const hasAnyRow = row != null;

  const cardState: CardState = isRunningHere
    ? {
        kind: "running",
        stage: stage === "summarizing" ? "summarizing" : "engine_starting",
      }
    : !row
      ? { kind: "empty" }
      : row.status === "ok"
        ? { kind: "ok", row }
        : { kind: "error", row };

  // 主按钮文案：优先级 跑中 > 已生成 > 未生成
  const mainBtnLabel = generating
    ? t("aiSummary.weekly.actions.stop")
    : hasOk
      ? t("aiSummary.weekly.actions.regenerate")
      : t("aiSummary.weekly.actions.start");

  // 周 pill 的人话标签：0/-1 走"本周/上周"，其它走 "MM-DD ~ MM-DD"
  const weekLabel = (off: number): string => {
    if (off === 0) return t("aiSummary.weekly.weekNav.thisWeek");
    if (off === -1) return t("aiSummary.weekly.weekNav.lastWeek");
    return `${toShortDate(monday)} ~ ${toShortDate(sunday)}`;
  };

  /** 把当前周的总结导出为 Markdown。比 daily 简化得多：单一段、无段间分隔。 */
  const onExportMarkdown = async () => {
    if (!row) return;
    const lines: string[] = ["---"];
    lines.push(
      `title: ${t("aiSummary.weekly.export.frontmatterTitle", {
        weekStart: monday,
        weekEnd: sunday,
      })}`,
    );
    lines.push(`week_start: ${monday}`);
    lines.push(`week_end: ${sunday}`);
    if (row.generatedAt) lines.push(`generated_at: ${row.generatedAt}`);
    if (row.model) lines.push(`model: ${row.model}`);
    lines.push(`status: ${row.status}`);
    lines.push("---", "");
    lines.push(
      `# ${t("aiSummary.weekly.export.heading", {
        weekStart: monday,
        weekEnd: sunday,
      })}`,
      "",
    );
    if (row.status === "ok") {
      lines.push(row.content?.trim() || t("aiSummary.weekly.export.stateEmptyContent"), "");
    } else {
      lines.push(t("aiSummary.weekly.export.stateError"), "");
      if (row.error) {
        lines.push(`> ${row.error.replace(/\n/g, "\n> ")}`, "");
      }
    }
    const md = lines.join("\n");
    const filename = `hindsight-weekly-${monday}.md`;

    let chosenPath: string | null = null;
    try {
      chosenPath = await save({
        title: t("aiSummary.weekly.actions.exportMarkdown"),
        defaultPath: filename,
        filters: [{ name: "Markdown", extensions: ["md"] }],
      });
    } catch (e) {
      setTopError(typeof e === "string" ? e : String(e));
      return;
    }
    if (!chosenPath) return;
    try {
      await invoke("write_text_file", { path: chosenPath, content: md });
      setTopNotice(t("aiSummary.weekly.toast.exported", { filename: chosenPath }));
      setTimeout(() => setTopNotice(null), 3500);
    } catch (e) {
      setTopError(
        typeof e === "string" ? e : `${t("aiSummary.weekly.errors.unknown")}：${String(e)}`,
      );
    }
  };

  return (
    <>
      <p className={styles.subtitle}>{t("aiSummary.weekly.subtitle")}</p>

      <header className={styles.header}>
        <div className={styles.weekNav}>
          <button
            ref={prevBtnRef}
            type="button"
            className={`${styles.navBtn} glow`}
            onClick={() => setWeekOffset((v) => v - 1)}
            disabled={generating}
            aria-label={t("aiSummary.weekly.weekNav.prevAria")}
          >
            <ChevronLeft size={14} strokeWidth={1.75} />
          </button>
          <button
            ref={pillRef}
            type="button"
            className={`${styles.weekPill} ${weekOffset !== 0 ? styles.weekPillClickable : ""} glow`}
            onClick={() => setWeekOffset(0)}
            disabled={generating || weekOffset === 0}
            title={
              weekOffset === 0
                ? undefined
                : t("aiSummary.weekly.weekNav.thisWeekBack")
            }
          >
            {weekLabel(weekOffset)}
          </button>
          <button
            ref={nextBtnRef}
            type="button"
            className={`${styles.navBtn} glow`}
            onClick={() => setWeekOffset((v) => v + 1)}
            disabled={generating || weekOffset >= 0}
            aria-label={t("aiSummary.weekly.weekNav.nextAria")}
          >
            <ChevronRight size={14} strokeWidth={1.75} />
          </button>
        </div>

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
            onClick={() => void onGenerate(hasOk)}
            disabled={!hasModel}
            title={
              hasModel
                ? hasOk
                  ? t("aiSummary.weekly.actions.regenerateTooltip")
                  : t("aiSummary.weekly.actions.startTooltip")
                : t("aiSummary.weekly.actions.noModelTooltip")
            }
          >
            {hasOk ? (
              <RefreshCcw size={14} strokeWidth={2} />
            ) : (
              <Play size={14} strokeWidth={2} />
            )}
            {mainBtnLabel}
          </button>
        )}

        <button
          type="button"
          className={styles.deleteBtn}
          onClick={() => {
            if (generating) return;
            setConfirmingDelete(true);
          }}
          disabled={generating || !hasAnyRow}
          title={
            !hasAnyRow
              ? t("aiSummary.weekly.actions.deleteEmptyTooltip")
              : t("aiSummary.weekly.actions.deleteTooltip")
          }
        >
          <Trash2 size={14} strokeWidth={2} />
          {t("aiSummary.weekly.actions.delete")}
        </button>

        <button
          type="button"
          className={styles.exportBtn}
          onClick={() => void onExportMarkdown()}
          disabled={generating || !hasOk}
          title={
            !hasOk
              ? t("aiSummary.weekly.actions.exportEmptyTooltip")
              : generating
                ? t("aiSummary.weekly.actions.exportRunningTooltip")
                : t("aiSummary.weekly.actions.exportTooltip")
          }
        >
          <Download size={14} strokeWidth={2} />
          {t("aiSummary.weekly.actions.exportMarkdown")}
        </button>
      </header>

      {topError ? (
        <div className={styles.errorBar}>
          <AlertTriangle size={14} strokeWidth={2.2} />
          <span>{topError}</span>
          <button
            type="button"
            className={styles.errorClose}
            onClick={clearTopError}
            aria-label={t("aiSummary.weekly.actions.dismissError")}
            title={t("aiSummary.weekly.actions.dismissError")}
          >
            <X size={12} strokeWidth={2.4} />
          </button>
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

      <div className={styles.card}>
        <div className={styles.cardHead}>
          <span className={styles.weekRange}>
            <Clock size={12} strokeWidth={2.2} />
            {monday} ~ {sunday}
          </span>
          <CardStatusBadge state={cardState} />
        </div>
        <CardBody state={cardState} />
      </div>

      <ConfirmDialog
        open={confirmingDelete}
        title={t("aiSummary.weekly.actions.deleteConfirmTitle", { weekStart: monday })}
        message={t("aiSummary.weekly.actions.deleteConfirmMessage")}
        variant="danger"
        onConfirm={async () => {
          setConfirmingDelete(false);
          try {
            await api.clearWeekSummary(monday);
            setRow(null);
            clearTopError();
          } catch (e) {
            setTopError(typeof e === "string" ? e : String(e));
          }
        }}
        onCancel={() => setConfirmingDelete(false)}
      />

      <ConfirmDialog
        open={pendingConfirm != null}
        title={
          pendingConfirm?.kind === "full"
            ? t("aiSummary.weekly.confirm.fullMissing.title")
            : t("aiSummary.weekly.confirm.partialMissing.title")
        }
        message={
          pendingConfirm == null
            ? ""
            : pendingConfirm.kind === "full"
              ? pendingConfirm.activityOnlyCount > 0
                ? t("aiSummary.weekly.confirm.fullMissing.message", {
                    activityCount: pendingConfirm.activityOnlyCount,
                  })
                : t("aiSummary.weekly.confirm.fullMissing.messageNoActivity")
              : t("aiSummary.weekly.confirm.partialMissing.message", {
                  count: pendingConfirm.missingDays.length,
                  dates: formatMissingDates(pendingConfirm.missingDays),
                })
        }
        confirmLabel={t("aiSummary.weekly.confirm.continueAnyway")}
        onConfirm={() => void onConfirmMissing()}
        onCancel={() => setPendingConfirm(null)}
      />
    </>
  );
}

function CardStatusBadge({ state }: { state: CardState }) {
  const { t } = useTranslation();
  switch (state.kind) {
    case "empty":
      return (
        <span className={`${styles.statusBadge} ${styles.statusEmpty}`}>
          {t("aiSummary.weekly.card.badge.empty")}
        </span>
      );
    case "running":
      return (
        <span className={`${styles.statusBadge} ${styles.statusRunning}`}>
          <Loader2 size={11} className={styles.spin} />
          {state.stage === "engine_starting"
            ? t("aiSummary.weekly.card.badge.engineStarting")
            : t("aiSummary.weekly.card.badge.summarizing")}
        </span>
      );
    case "ok":
      return (
        <span className={`${styles.statusBadge} ${styles.statusOk}`}>
          {t("aiSummary.weekly.card.badge.ok")}
        </span>
      );
    case "error":
      return (
        <span className={`${styles.statusBadge} ${styles.statusError}`}>
          {t("aiSummary.weekly.card.badge.error")}
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
          {t("aiSummary.weekly.card.body.empty")}
        </div>
      );
    case "running":
      return (
        <div className={styles.bodyMuted}>
          {state.stage === "engine_starting"
            ? t("aiSummary.weekly.card.body.engineStarting")
            : t("aiSummary.weekly.card.body.summarizing")}
        </div>
      );
    case "ok":
      return <div className={styles.bodyText}>{state.row.content}</div>;
    case "error":
      return (
        <div className={styles.bodyError}>
          <strong>{t("aiSummary.weekly.card.body.errorLabel")}</strong>
          {state.row.error || t("aiSummary.weekly.errors.unknown")}
        </div>
      );
  }
}
