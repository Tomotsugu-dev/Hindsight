import { useEffect, useMemo, useRef, useState, useSyncExternalStore } from "react";
import { useTranslation } from "react-i18next";
import { useNavigate } from "react-router-dom";
import { save } from "@tauri-apps/plugin-dialog";
import { AlertCircle, Download, Trash2, X } from "lucide-react";
import { logError } from "../../../lib/logger";
import { localizeAiError } from "../../../lib/aiErrors";
import { ROUTES } from "../../../config/nav";
import { useDurationFormatter } from "../../../utils/duration";
import { resolveSegmentDotColor } from "../../../utils/segmentColor";
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
  type AiSegment,
  type SegmentSummaryRow,
  type WeekPrecheckDay,
  type WeekPrecheckResp,
} from "../../../api/hindsight";
import { MarkdownText } from "../../../components/MarkdownText/MarkdownText";
import { IconButton, MainButton, NoticeBars, PeriodNav, ReportHeader } from "./ReportControls";
import ctl from "./ReportControls.module.css";
import { SegmentTimeline, SegmentTimelineRow } from "./SegmentTimeline";
import tl from "./SegmentTimeline.module.css";
import {
  compactDuration,
  formatGeneratedAt,
  heatLevel,
  modelLabel,
  parseDateStr,
  toDateStr,
  weekDayKind,
  type WeekDayKind,
} from "./reportFormat";
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

/** 把 "YYYY-MM-DD" 转成 "MM-DD" 简写（缺失日确认框里用）。 */
function toShortDate(s: string): string {
  return s.length === 10 ? s.slice(5) : s;
}

/** 这次按下是不是按在 `el` 自己的滚动条上。 */
function onScrollbar(el: Element, e: PointerEvent): boolean {
  const vertical = el.scrollHeight > el.clientHeight && e.offsetX >= el.clientWidth;
  const horizontal = el.scrollWidth > el.clientWidth && e.offsetY >= el.clientHeight;
  return vertical || horizontal;
}

/** 卡片当前展示状态——由"是否在跑" + 落库 row 推导。 */
type CardState =
  | { kind: "empty" }
  | { kind: "running" }
  | { kind: "ok"; row: SegmentSummaryRow }
  | { kind: "error"; row: SegmentSummaryRow };

/** 热力图里的一格。kind 为 null：还没拿到这天有没有日报。 */
interface DayCell {
  date: string;
  weekday: string;
  /** 给读屏和面板标题用的完整日期，如「11月4日星期三」 */
  longDate: string;
  dayNum: number;
  kind: WeekDayKind | null;
  /** 当天的活动分钟数；还没拿到是 null */
  minutes: number | null;
  /** 热力深浅，只有有日报的天上色 */
  level: number;
  isToday: boolean;
}

/**
 * 周报 tab：上面七天热力图，下面默认是整周回顾，点某一天换成那天的日报。
 *
 * - 热力图：切周时调 [`api.precheckWeekSummary`] 看哪天有日报，调 [`api.getWeekDays`]
 *   拿每天的活动时长；有日报的天按时长上色。读不到时格子只画日期
 * - 点没有日报的天：显示空，按钮跳到日报页打开那一天去生成
 * - 点格子和面板以外的地方、或按 Esc，回到整周回顾
 * - 点击「开始/重新生成」时再调一次 precheck 看本周日报覆盖情况：
 *   全有 → 直接生成；部分 / 整周缺 → 弹 ConfirmDialog 让用户选"继续生成"，
 *   确认后以 `allowMissingDays=true` 跑后端（缺日报的天用当日 top apps 顶替进 prompt）
 * - 失败原因常见两类：模型未配置 + "本周完全没数据"——后者由后端写 error 行，
 *   前端按错误展示
 */
export default function WeeklyTab() {
  const { t, i18n } = useTranslation();
  const { settings } = useSettings();
  const navigate = useNavigate();
  const fmtDuration = useDurationFormatter();
  const [weekOffset, setWeekOffset] = useState(0);
  const [row, setRow] = useState<SegmentSummaryRow | null>(null);
  const [days, setDays] = useState<WeekPrecheckDay[] | null>(null);
  // 每天的活动分钟数，按 "YYYY-MM-DD"
  const [minutesByDate, setMinutesByDate] = useState<Map<string, number> | null>(null);
  // 点中的那天；null = 看整周回顾
  const [selected, setSelected] = useState<string | null>(null);
  // 点中那天的日报；null = 还在读
  const [dayRows, setDayRows] = useState<SegmentSummaryRow[] | null>(null);
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

  const { monday, sunday } = useMemo(() => weekRangeFromOffset(weekOffset), [weekOffset]);

  // hasModel：周报走纯文本段总结——本地 summary 模型或选定云端 二选一即可。
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
  // "继续生成"误用 A 周的缺失日列表跑 B 周的现象。点中的那天也一起清掉
  useEffect(() => {
    setPendingConfirm(null);
    setSelected(null);
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

  // 切周 / 进页：拉热力图的数据——哪天有日报、每天多少活动。读不到不报错，格子只画日期
  useEffect(() => {
    let cancelled = false;
    setDays(null);
    setMinutesByDate(null);
    api
      .precheckWeekSummary(monday)
      .then((r) => {
        if (!cancelled) setDays(r.days);
      })
      .catch((e) => logError("weekly.days", e));
    api
      .getWeekDays(weekOffset)
      .then((list) => {
        if (cancelled) return;
        const m = new Map<string, number>();
        list.forEach((d) => {
          const secs = d.segments.reduce((sum, s) => sum + s.secs, 0);
          m.set(d.date, Math.round(secs / 60));
        });
        setMinutesByDate(m);
      })
      .catch((e) => logError("weekly.minutes", e));
    return () => {
      cancelled = true;
    };
  }, [monday, weekOffset]);

  // 点中某天：读那天的日报
  useEffect(() => {
    if (!selected) return;
    let cancelled = false;
    setDayRows(null);
    api
      .getDaySummary(selected)
      .then((list) => {
        if (!cancelled) setDayRows([...list].sort((a, b) => a.segmentIdx - b.segmentIdx));
      })
      .catch((e) => {
        if (cancelled) return;
        logError("weekly.daySummary", e);
        setDayRows([]);
      });
    return () => {
      cancelled = true;
    };
  }, [selected]);

  // 点中某天时，点格子和面板以外的地方、或按 Esc，回到整周回顾
  const heatRef = useRef<HTMLUListElement>(null);
  const panelRef = useRef<HTMLElement>(null);
  useEffect(() => {
    if (!selected) return;
    // 用按下而不用 click：在面板里拖选文字、松手在面板外，不该关掉
    const onPointerDown = (e: PointerEvent) => {
      if (e.button !== 0) return;
      const target = e.target;
      if (!(target instanceof Element)) return;
      // 读长日报时要拖滚动条
      if (onScrollbar(target, e)) return;
      if (panelRef.current?.contains(target)) return;
      // 格子自己的点击负责切换
      if (heatRef.current?.contains(target.closest("button"))) return;
      setSelected(null);
    };
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.key === "Escape") setSelected(null);
    };
    document.addEventListener("pointerdown", onPointerDown);
    document.addEventListener("keydown", onKeyDown);
    return () => {
      document.removeEventListener("pointerdown", onPointerDown);
      document.removeEventListener("keydown", onKeyDown);
    };
  }, [selected]);

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
  const formatMissingDates = (missing: WeekPrecheckDay[]): string =>
    missing.map((d) => `${toShortDate(d.date)} ${d.weekday}`).join(", ");

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
    ? { kind: "running" }
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

  // 周切换按钮上的字，跟周统计同一套说法；具体日期放在状态行开头
  const navLabel =
    weekOffset === 0
      ? t("week.weekNav.thisWeek")
      : weekOffset === -1
        ? t("week.weekNav.lastWeek")
        : t("week.weekNav.weeksAgo", { count: -weekOffset });
  const weekText = useMemo(() => {
    const fmt = new Intl.DateTimeFormat(i18n.language, { month: "short", day: "numeric" });
    return `${fmt.format(parseDateStr(monday))} – ${fmt.format(parseDateStr(sunday))}`;
  }, [monday, sunday, i18n.language]);

  // 热力图的七格
  const today = toDateStr(new Date());
  const cells = useMemo((): DayCell[] => {
    const weekdayFmt = new Intl.DateTimeFormat(i18n.language, { weekday: "short" });
    const longFmt = new Intl.DateTimeFormat(i18n.language, {
      month: "long",
      day: "numeric",
      weekday: "short",
    });
    // 分级按这周最忙的一天，只看有日报的天（只有它们上色）
    const dailyDates = new Set(
      (days ?? []).filter((d) => d.hasDaily && d.date <= today).map((d) => d.date),
    );
    const weekMax = Math.max(
      0,
      ...[...dailyDates].map((date) => minutesByDate?.get(date) ?? 0),
    );
    return Array.from({ length: 7 }, (_, i) => {
      const d = parseDateStr(monday);
      d.setDate(d.getDate() + i);
      const date = toDateStr(d);
      const info = days?.find((x) => x.date === date);
      // 没拿到数据时，未来的日子照样能判断
      const kind = info ? weekDayKind(info, today) : date > today ? "future" : null;
      const minutes = minutesByDate ? (minutesByDate.get(date) ?? 0) : null;
      return {
        date,
        weekday: weekdayFmt.format(d),
        longDate: longFmt.format(d),
        dayNum: d.getDate(),
        kind,
        minutes,
        // 有日报的天至少 1 级，0 分钟也看得出上了色
        level: kind === "daily" ? Math.max(1, heatLevel(minutes ?? 0, weekMax)) : 0,
        isToday: date === today,
      };
    });
  }, [monday, days, minutesByDate, today, i18n.language]);

  // 已经过去的日子（含今天）里有几天有日报
  const pastCells = cells.filter((c) => c.kind !== null && c.kind !== "future");
  const dailyCount = pastCells.filter((c) => c.kind === "daily").length;
  const selectedCell = cells.find((c) => c.date === selected) ?? null;

  /** 跳到日报页并打开这一天。 */
  const openDaily = (date: string) => navigate(`${ROUTES.aiSummary}?date=${date}`);

  // 工具栏下面的状态行：在跑时说到哪一步，没生成时一句话，生成了说什么时候、用的哪个模型
  const status = ((): { busy: boolean; text: string } => {
    if (generating) {
      const text =
        enginePhase ??
        (stage === "summarizing"
          ? t("aiSummary.weekly.card.body.summarizing")
          : t("aiSummary.weekly.card.body.engineStarting"));
      return { busy: true, text };
    }
    if (!row) {
      return {
        busy: false,
        text: hasModel
          ? t("aiSummary.weekly.status.empty")
          : t("aiSummary.weekly.actions.noModelTooltip"),
      };
    }
    if (row.status !== "ok") return { busy: false, text: t("aiSummary.weekly.status.failed") };
    const parts: string[] = [];
    const at = formatGeneratedAt(row.generatedAt, i18n.language);
    if (at) parts.push(t("aiSummary.weekly.status.generated", { time: at }));
    if (row.model) parts.push(modelLabel(row.model));
    return { busy: false, text: parts.join(" · ") };
  })();

  // 还没生成时正文里的提示：日报太少就先劝去补日报
  const emptyHint =
    days === null
      ? t("aiSummary.weekly.empty.ready")
      : dailyCount === 0
        ? t("aiSummary.weekly.empty.noDaily")
        : dailyCount < Math.min(3, pastCells.length)
          ? t("aiSummary.weekly.empty.fewDaily", { count: dailyCount })
          : t("aiSummary.weekly.empty.ready");

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
      await api.writeTextFile(chosenPath, md);
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
      <ReportHeader
        subtitle={t("aiSummary.weekly.subtitle")}
        period={weekText}
        status={status}
        nav={
          <PeriodNav
            groupLabel={t("aiSummary.weekly.weekNav.groupAria")}
            prevLabel={t("aiSummary.weekly.weekNav.prevAria")}
            nextLabel={t("aiSummary.weekly.weekNav.nextAria")}
            resetTitle={t("aiSummary.weekly.weekNav.thisWeekBack")}
            label={navLabel}
            atCurrent={weekOffset >= 0}
            locked={generating}
            onPrev={() => setWeekOffset((v) => v - 1)}
            onNext={() => setWeekOffset((v) => v + 1)}
            onReset={() => setWeekOffset(0)}
          />
        }
        actions={
          <>
            {generating ? (
              <MainButton action="stop" label={mainBtnLabel} onClick={() => void onCancel()} />
            ) : (
              <MainButton
                action={hasOk ? "regenerate" : "start"}
                label={mainBtnLabel}
                onClick={() => void onGenerate(hasOk)}
                disabled={!hasModel}
                title={
                  hasModel
                    ? hasOk
                      ? t("aiSummary.weekly.actions.regenerateTooltip")
                      : t("aiSummary.weekly.actions.startTooltip")
                    : t("aiSummary.weekly.actions.noModelTooltip")
                }
              />
            )}
            <IconButton
              label={t("aiSummary.weekly.actions.exportMarkdown")}
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
              <Download size={16} strokeWidth={2} />
            </IconButton>
            <IconButton
              label={t("aiSummary.weekly.actions.delete")}
              danger
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
              <Trash2 size={16} strokeWidth={2} />
            </IconButton>
          </>
        }
      />

      <NoticeBars
        error={topError}
        dismissLabel={t("aiSummary.weekly.actions.dismissError")}
        onDismissError={clearTopError}
        notice={topNotice}
      />

      {/* 热力图：有日报的天按活动时长上色，点一下在下面看那天的日报，再点回到整周回顾 */}
      <ul ref={heatRef} className={styles.heat} aria-label={t("aiSummary.weekly.days.aria")}>
        {cells.map((c) => {
          const parts = [
            c.longDate,
            c.kind ? t(`aiSummary.weekly.days.${c.kind}`) : null,
            c.kind !== "future" && c.minutes ? fmtDuration(c.minutes) : null,
          ];
          const label = parts.filter(Boolean).join(" · ");
          return (
            <li key={c.date}>
              <button
                type="button"
                className={styles.tile}
                data-kind={c.kind ?? "unknown"}
                data-level={c.level}
                aria-pressed={selected === c.date}
                aria-label={label}
                title={label}
                disabled={c.kind === "future"}
                onClick={() => setSelected((v) => (v === c.date ? null : c.date))}
              >
                <span className={styles.tileWeekday}>{c.weekday}</span>
                <span className={styles.tileDate}>{c.dayNum}</span>
                <span className={styles.tileTime}>
                  {c.kind !== "future" && c.minutes ? compactDuration(c.minutes) : ""}
                </span>
                {c.isToday ? <span className={styles.todayDot} aria-hidden /> : null}
              </button>
            </li>
          );
        })}
      </ul>

      <section ref={panelRef} className={styles.panel} data-part="card">
        <header className={styles.panelHead}>
          <h3 className={styles.panelTitle}>
            {selectedCell
              ? t("aiSummary.weekly.panel.day", { date: selectedCell.longDate })
              : t("aiSummary.weekly.panel.week")}
          </h3>
          {selectedCell ? (
            <div className={styles.panelActions}>
              {dayRows && dayRows.length > 0 ? (
                <button
                  type="button"
                  className={ctl.textBtn}
                  onClick={() => openDaily(selectedCell.date)}
                >
                  {t("aiSummary.weekly.panel.openDaily")}
                </button>
              ) : null}
              <IconButton
                small
                label={t("aiSummary.weekly.panel.back")}
                title={t("aiSummary.weekly.panel.back")}
                onClick={() => setSelected(null)}
              >
                <X size={15} strokeWidth={2} />
              </IconButton>
            </div>
          ) : null}
        </header>
        {/* key 换了就重新挂载，切换时淡入一次 */}
        <div className={styles.panelBody} key={selected ?? "week"}>
          {selectedCell ? (
            <DayBody
              cell={selectedCell}
              rows={dayRows}
              segments={settings?.ai.segments ?? []}
              onGenerate={() => openDaily(selectedCell.date)}
            />
          ) : (
            <WeekBody state={cardState} emptyHint={emptyHint} />
          )}
        </div>
      </section>

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

interface DayBodyProps {
  cell: DayCell;
  /** 那天的日报；null = 还在读 */
  rows: SegmentSummaryRow[] | null;
  /** 设置里的时段，取圆点颜色 */
  segments: AiSegment[];
  onGenerate: () => void;
}

/** 点中某一天时面板里的内容：那天的日报按时段列出；没有日报就显示空和生成按钮。 */
function DayBody({ cell, rows, segments, onGenerate }: DayBodyProps) {
  const { t } = useTranslation();
  const fmtDuration = useDurationFormatter();
  if (rows === null) {
    return (
      <div className={ctl.skeleton} aria-hidden>
        <span />
        <span />
        <span />
      </div>
    );
  }
  if (rows.length === 0) {
    return (
      <div className={styles.dayEmpty}>
        <p className={styles.dayEmptyTitle}>{t("aiSummary.weekly.panel.dayEmpty")}</p>
        <p className={styles.dayEmptyNote}>
          {cell.minutes
            ? t("aiSummary.weekly.panel.dayActivity", { duration: fmtDuration(cell.minutes) })
            : t("aiSummary.weekly.panel.dayNoActivity")}
        </p>
        <MainButton
          action="start"
          label={t("aiSummary.weekly.panel.generateDay")}
          onClick={onGenerate}
        />
      </div>
    );
  }
  return (
    <SegmentTimeline className={styles.dayTimeline}>
      {rows.map((r) => (
        <SegmentTimelineRow
          key={r.segmentIdx}
          startHour={r.startHour}
          endHour={r.endHour}
          label={r.label}
          color={resolveSegmentDotColor({
            startHour: r.startHour,
            endHour: r.endHour,
            color: segments[r.segmentIdx]?.color ?? "",
          })}
          hollow={r.status === "skipped_no_activity" || r.status === "skipped_no_screenshots"}
          heading="h4"
        >
          {r.status === "ok" ? (
            <MarkdownText text={r.content} className={tl.bodyText} />
          ) : r.status === "skipped_no_activity" ? (
            <p className={tl.bodyMuted}>{t("aiSummary.daily.card.body.skippedNoActivity")}</p>
          ) : r.status === "skipped_no_screenshots" ? (
            <p className={tl.bodyMuted}>{t("aiSummary.daily.card.body.skipped")}</p>
          ) : (
            <div className={ctl.errorBox}>
              <AlertCircle size={14} strokeWidth={2} />
              <span className={ctl.errorText}>
                <strong>{t("aiSummary.daily.card.body.errorLabel")}</strong>
                {r.error ? localizeAiError(r.error) : t("aiSummary.daily.errors.unknown")}
              </span>
            </div>
          )}
        </SegmentTimelineRow>
      ))}
    </SegmentTimeline>
  );
}

/** 面板默认的内容：整周回顾正文，或者生成中、失败、还没生成时的样子。 */
function WeekBody({ state, emptyHint }: { state: CardState; emptyHint: string }) {
  const { t } = useTranslation();
  switch (state.kind) {
    case "empty":
      return <p className={styles.bodyMuted}>{emptyHint}</p>;
    case "running":
      return (
        <div className={ctl.skeleton} aria-hidden>
          <span />
          <span />
          <span />
          <span />
          <span />
        </div>
      );
    case "ok":
      return <MarkdownText text={state.row.content} className={styles.bodyText} />;
    case "error":
      return (
        <div className={ctl.errorBox}>
          <AlertCircle size={14} strokeWidth={2} />
          <span className={ctl.errorText}>
            <strong>{t("aiSummary.weekly.card.body.errorLabel")}</strong>
            {state.row.error
              ? localizeAiError(state.row.error)
              : t("aiSummary.weekly.errors.unknown")}
          </span>
        </div>
      );
  }
}
