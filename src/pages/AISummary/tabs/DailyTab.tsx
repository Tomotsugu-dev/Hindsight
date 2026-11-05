import { useEffect, useMemo, useRef, useState, useSyncExternalStore } from "react";
import { useTranslation } from "react-i18next";
import { useSearchParams } from "react-router-dom";
import { save } from "@tauri-apps/plugin-dialog";
import { logError } from "../../../lib/logger";
import { localizeAiError } from "../../../lib/aiErrors";
import { AlertCircle, Download, Loader2, Pencil, RotateCcw, Trash2 } from "lucide-react";
import {
  api,
  SUMMARY_CLOUD_SENTINEL,
  type AiSegment,
  type SegmentSummaryRow,
} from "../../../api/hindsight";
import { useSettings } from "../../../state/settings";
import { ConfirmDialog } from "../../../components/ConfirmDialog/ConfirmDialog";
import { MarkdownText } from "../../../components/MarkdownText/MarkdownText";
import { resolveSegmentDotColor } from "../../../utils/segmentColor";
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
import { IconButton, MainButton, NoticeBars, PeriodNav, ReportHeader } from "./ReportControls";
import ctl from "./ReportControls.module.css";
import { SegmentTimeline, SegmentTimelineRow } from "./SegmentTimeline";
import tl from "./SegmentTimeline.module.css";
import { SummaryEditor } from "./SummaryEditor";
import { dayOffsetOf, formatGeneratedAt, modelLabel, parseDateStr, toDateStr } from "./reportFormat";
import styles from "./DailyTab.module.css";

/** 段卡片在 UI 里的展示状态——由 ai_summaries 行 + 当前 in-flight 进度推导。 */
type CardState =
  | { kind: "empty" } // 还没生成 / 还没轮到
  | { kind: "running" } // 段总结生成中
  | { kind: "ok"; row: SegmentSummaryRow }
  | { kind: "skipped"; row: SegmentSummaryRow }
  | { kind: "error"; row: SegmentSummaryRow };

/** 把 dayOffset (0=今天 / -1=昨天 / ...) 转 "YYYY-MM-DD" 本地日期字符串。 */
function offsetToDateStr(dayOffset: number): string {
  const d = new Date();
  d.setDate(d.getDate() + dayOffset);
  return toDateStr(d);
}

/**
 * 日报 tab：当前一天的活动按段汇总 + 渐次出文。
 *
 * 进度通过 listen `ai://summary-progress` 流式更新；状态机：
 *   empty → running → (ok | skipped | error)
 */
export default function DailyTab() {
  const { t, i18n } = useTranslation();
  const { settings, reload: reloadSettings } = useSettings();
  // 从周报点某一天跳过来时带 ?date=YYYY-MM-DD，打开那一天
  const [searchParams] = useSearchParams();
  const [dayOffset, setDayOffset] = useState(() => dayOffsetOf(searchParams.get("date")));
  const [rows, setRows] = useState<Map<number, SegmentSummaryRow>>(new Map());
  // 在跑状态全部从 module-level store 读，组件 unmount 不丢；切走再回来按钮、
  // 状态行、当前段 spinner 都自动恢复。
  const runSnap = useSyncExternalStore(
    subscribeDailySummary,
    getDailyRunningSnapshot,
    getDailyRunningSnapshot,
  );
  // 导出 / 删除等操作完成后短暂显示的成功提示，3s 后自清
  const [topNotice, setTopNotice] = useState<string | null>(null);
  const [confirmingDelete, setConfirmingDelete] = useState(false);
  // 正在修改的段；null = 没在改
  const [editingIdx, setEditingIdx] = useState<number | null>(null);

  const date = useMemo(() => offsetToDateStr(dayOffset), [dayOffset]);
  // 状态行开头的「11月4日周三」：按界面语言排
  const dateText = useMemo(
    () =>
      new Intl.DateTimeFormat(i18n.language, {
        month: "long",
        day: "numeric",
        weekday: "short",
      }).format(parseDateStr(date)),
    [date, i18n.language],
  );
  const segments = settings?.ai.segments ?? [];
  const activeMain = settings?.ai.activeMain ?? "";
  // hasModel 跟后端 summary_runner 的 check 对齐：段总结满足以下任一：
  //   - 用户在云端卡选 Text（rawSummaryMain == sentinel）且云端 API 启用
  //   - 用户在本地某个模型卡选 Text（rawSummaryMain 是真实文件名）
  //   - 都没选但 activeMain 存在（旧版 fallback 兼容）
  const rawSummaryMain = settings?.ai.summaryMain ?? "";
  const externalEnabled = settings?.ai.externalEnabled ?? false;
  const cloudRoute = externalEnabled && rawSummaryMain === SUMMARY_CLOUD_SENTINEL;
  // 云端总结真正可跑 = 路由选中云端 + 三元组配齐(endpoint/model 任一缺失时
  // 后端 External 分支会直接报错,不该拿"下载本地引擎"去引导)
  const cloudConfigured =
    externalEnabled &&
    (settings?.ai.endpoint ?? "").trim() !== "" &&
    (settings?.ai.model ?? "").trim() !== "";
  const cloudSummaryReady = cloudRoute && cloudConfigured;
  const localSummaryAvailable =
    (rawSummaryMain !== "" && rawSummaryMain !== SUMMARY_CLOUD_SENTINEL) ||
    activeMain !== "";
  const hasModel = cloudRoute || localSummaryAvailable;

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
    setEditingIdx(null);
    api
      .getDaySummary(date)
      .then((list) => {
        if (cancelled) return;
        const m = new Map<number, SegmentSummaryRow>();
        list.forEach((r) => m.set(r.segmentIdx, r));
        setRows(m);
      })
      .catch((e) => {
        if (cancelled) return;
        logError("daily.getSummary", e);
        // 旧实现只 logError，前端页空白用户没线索；现在把错误显式推顶：
        // 进页就 fetch 不到说明 DB 出问题了或后端 command 挂了——不该让用户对着
        // 一片空白干瞪眼。setTopError 让 DailyTab 顶部红条显示错误，用户能立刻
        // 知道是"读 DB 失败"而不是"还没生成"。
        setTopError(typeof e === "string" ? e : String(e));
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

  // AI 引擎缺失时的延后动作——保存用户原本要做的"生成日报 / 重试段"，
  // 待 ConfirmDialog 确认 + downloadBinary 跑完后再执行。null = 没有 pending。
  const [pendingAfterDownload, setPendingAfterDownload] =
    useState<null | (() => Promise<void>)>(null);

  /** 本地路由时检查 llama.cpp 引擎是否已装；齐 → 直接跑 action；
   *  缺 → 保存 action、弹 ConfirmDialog 引导下载，confirm 后下完再续上。
   *  云端总结配齐并选中时不需要任何本地组件，直接放行；
   *  onnxruntime 是屏幕记忆 OCR 的依赖，日报单步管线用不到，这里不查。 */
  const requireEngineThen = async (action: () => Promise<void>) => {
    if (!cloudSummaryReady) {
      try {
        const status = await api.getEngineStatus();
        if (!status.installed) {
          // useState setter 接函数会被当成"updater"立即调用——用 () => fn 包一层
          setPendingAfterDownload(() => action);
          return;
        }
      } catch (e) {
        // getEngineStatus 失败 → 保守起见直接 fallthrough 跑 action，让原来的错处理生效
        logError("DailyTab.requireEngineThen.status", e);
      }
    }
    await action();
  };

  const onConfirmDownloadAi = async () => {
    const action = pendingAfterDownload;
    setPendingAfterDownload(null);
    if (!action) return;
    try {
      if (cloudConfigured) {
        // 云端 API 已配齐但总结槽位没选云端——用户点「使用云端生成」:
        // 把总结切到云端(等价于模型页云端卡点 Text)后直接继续生成
        await api.setStepModel("summary", SUMMARY_CLOUD_SENTINEL, null);
        await reloadSettings();
      } else {
        await api.downloadBinary();
      }
      await action();
    } catch (e) {
      setTopError(typeof e === "string" ? e : String(e));
    }
  };

  const onGenerate = async (forceRefresh: boolean) => {
    if (!hasModel) {
      setTopError(t("aiSummary.daily.errors.noModel"));
      return;
    }
    // 重新点开始/再生成时主动清旧错误条——否则用户上一轮失败留着的红条
    // 跟新一轮的进度提示混在一起容易误以为新一轮也失败了
    clearTopError();
    await requireEngineThen(async () => {
      try {
        await startDailyGenerate(date, forceRefresh);
      } catch {
        // store 已经设过 topError；这里不再重复处理
      }
    });
  };

  const onCancel = async () => {
    await cancelDailyGenerate();
  };

  const onRetrySegment = async (segmentIdx: number) => {
    if (!hasModel) {
      setTopError(t("aiSummary.daily.errors.noModel"));
      return;
    }
    if (runSnap.generating) return;
    clearTopError();
    await requireEngineThen(async () => {
      try {
        await retryDailySegment(date, segmentIdx);
      } catch {
        // store 已经设过 topError
      }
    });
  };

  // 是否在跑 = store 标 generating + 跑的日期跟当前 view 日期一致；
  // 切到别的日期看时按钮回到"开始总结"，不该被另一天的 run 拖累
  const isRunningHere =
    runSnap.generating && runSnap.runningDate === date;
  const runningIdx = isRunningHere ? runSnap.runningIdx : null;
  const enginePhase = isRunningHere ? runSnap.enginePhase : null;
  const topError = runSnap.topError;
  const generating = isRunningHere;
  const stopping = isRunningHere && runSnap.stopping;

  // 开始生成就收起编辑框：这一轮会覆盖正在改的段
  useEffect(() => {
    if (generating) setEditingIdx(null);
  }, [generating]);

  const onSaveEdit = async (idx: number, text: string) => {
    try {
      const row = await api.editSummarySegment("daily", date, idx, text);
      // 保存期间切到了别的日期：不能把这一行放进新日期
      if (row.localDate !== dateRef.current) return;
      setRows((prev) => new Map(prev).set(idx, row));
      setEditingIdx(null);
    } catch (e) {
      setTopError(typeof e === "string" ? e : String(e));
    }
  };

  const allDone = segments.length > 0 && segments.every((_, i) => rows.has(i));
  const mainBtnLabel = generating
    ? stopping
      ? t("aiSummary.daily.actions.stopping")
      : t("aiSummary.daily.actions.stop")
    : allDone
      ? t("aiSummary.daily.actions.regenerate")
      : t("aiSummary.daily.actions.start");

  // 至少有一段已落库（不论 ok / skipped / error）就允许导出
  const hasAnyRow = rows.size > 0;

  // 日期切换按钮上的字，跟日统计同一套说法；具体日期放在状态行开头
  const navLabel =
    dayOffset === 0
      ? t("today.dayNav.today")
      : dayOffset === -1
        ? t("today.dayNav.yesterday")
        : t("today.dayNav.daysAgo", { count: -dayOffset });

  // 工具栏下面的状态行：在跑时说跑到哪，没数据时给提示，有数据时说生成了几段、
  // 最近一段什么时候生成、用的哪个模型
  const status = ((): { busy: boolean; text: string } => {
    if (generating) {
      const seg = runningIdx != null ? segments[runningIdx] : undefined;
      let text: string;
      if (stopping) text = t("aiSummary.daily.actions.stopping");
      else if (enginePhase) text = enginePhase;
      else if (seg && runningIdx != null)
        text = t("aiSummary.daily.status.running", {
          label: seg.label,
          index: runningIdx + 1,
          total: segments.length,
        });
      else text = t("aiSummary.daily.card.badge.summarizing");
      return { busy: true, text };
    }
    if (!hasAnyRow) {
      return {
        busy: false,
        text: hasModel
          ? t("aiSummary.daily.status.empty")
          : t("aiSummary.daily.actions.noModelTooltip"),
      };
    }
    // 写出了内容或确认没活动的段算完成；失败的段不算
    let finished = 0;
    segments.forEach((_, i) => {
      const status = rows.get(i)?.status;
      if (
        status === "ok" ||
        status === "skipped_no_screenshots" ||
        status === "skipped_no_activity"
      )
        finished += 1;
    });
    let latest: SegmentSummaryRow | null = null;
    for (const r of rows.values()) {
      if (!latest || r.generatedAt > latest.generatedAt) latest = r;
    }
    const parts = [
      finished === segments.length
        ? t("aiSummary.daily.status.done", { count: finished })
        : t("aiSummary.daily.status.partial", { done: finished, total: segments.length }),
    ];
    const at = latest ? formatGeneratedAt(latest.generatedAt, i18n.language, date) : null;
    if (at) parts.push(at);
    if (latest?.model) parts.push(modelLabel(latest.model));
    return { busy: false, text: parts.join(" · ") };
  })();

  /** 把当前日期的所有段总结拼成 Markdown 文件并下载。
   *  含 YAML frontmatter（date / generated_at / model / 状态汇总）+ 顶部总览
   *  + 每段状态徽章 + 段间 --- 分隔，方便贴 Notion / Obsidian / GitHub 渲染。
   *  导出文本本身随当前 i18n 语言生成。 */
  const onExportMarkdown = async () => {
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
      else if (
        row.status === "skipped_no_screenshots" ||
        row.status === "skipped_no_activity"
      )
        skipCount += 1;
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
      } else if (row.status === "skipped_no_activity") {
        lines.push(t("aiSummary.daily.export.stateSkippedNoActivity"), "");
      } else {
        lines.push(t("aiSummary.daily.export.stateError"), "");
        lines.push(
          `> ${(row.error ? localizeAiError(row.error) : t("aiSummary.daily.errors.unknown")).replace(/\n/g, "\n> ")}`,
          "",
        );
      }

      // 段间分隔（最后一段不加）
      if (idx < segments.length - 1) {
        lines.push("---", "");
      }
    });

    const md = lines.join("\n");
    const filename = `hindsight-daily-${date}.md`;

    // Tauri webview 不可靠支持浏览器原生 `<a download>`——改用 Tauri save dialog
    // 让用户选位置 + 后端 std::fs 写文件 + 顶部绿色 successBar 显示落盘绝对路径。
    let chosenPath: string | null = null;
    try {
      chosenPath = await save({
        title: t("aiSummary.daily.actions.exportMarkdown"),
        defaultPath: filename,
        filters: [{ name: "Markdown", extensions: ["md"] }],
      });
    } catch (e) {
      setTopError(typeof e === "string" ? e : String(e));
      return;
    }
    if (!chosenPath) return; // 用户取消
    try {
      await api.writeTextFile(chosenPath, md);
      setTopNotice(t("aiSummary.daily.toast.exported", { filename: chosenPath }));
      setTimeout(() => setTopNotice(null), 3500);
    } catch (e) {
      setTopError(
        typeof e === "string" ? e : `${t("aiSummary.daily.errors.unknown")}：${String(e)}`,
      );
    }
  };

  return (
    <>
      <ReportHeader
        subtitle={t("aiSummary.daily.subtitle")}
        period={dateText}
        status={status}
        nav={
          <PeriodNav
            groupLabel={t("aiSummary.daily.dateNav.groupAria")}
            prevLabel={t("aiSummary.daily.dateNav.prevAria")}
            nextLabel={t("aiSummary.daily.dateNav.nextAria")}
            resetTitle={t("aiSummary.daily.dateNav.todayBack")}
            label={navLabel}
            atCurrent={dayOffset >= 0}
            locked={generating}
            onPrev={() => setDayOffset((v) => v - 1)}
            onNext={() => setDayOffset((v) => v + 1)}
            onReset={() => setDayOffset(0)}
          />
        }
        actions={
          <>
            {generating ? (
              <MainButton
                action="stop"
                label={mainBtnLabel}
                onClick={() => void onCancel()}
                disabled={stopping}
              />
            ) : (
              <MainButton
                action={allDone ? "regenerate" : "start"}
                label={mainBtnLabel}
                onClick={() => void onGenerate(allDone)}
                disabled={!hasModel}
                title={
                  hasModel
                    ? allDone
                      ? t("aiSummary.daily.actions.regenerateTooltip")
                      : t("aiSummary.daily.actions.startTooltip")
                    : t("aiSummary.daily.actions.noModelTooltip")
                }
              />
            )}
            <IconButton
              label={t("aiSummary.daily.actions.exportMarkdown")}
              onClick={() => void onExportMarkdown()}
              disabled={generating || !hasAnyRow}
              title={
                !hasAnyRow
                  ? t("aiSummary.daily.actions.exportEmptyTooltip")
                  : generating
                    ? t("aiSummary.daily.actions.exportRunningTooltip")
                    : t("aiSummary.daily.actions.exportTooltip")
              }
            >
              <Download size={16} strokeWidth={2} />
            </IconButton>
            <IconButton
              label={t("aiSummary.daily.actions.delete")}
              danger
              onClick={() => {
                if (generating) return;
                setConfirmingDelete(true);
              }}
              disabled={generating || !hasAnyRow}
              title={
                !hasAnyRow
                  ? t("aiSummary.daily.actions.deleteEmptyTooltip")
                  : t("aiSummary.daily.actions.deleteTooltip")
              }
            >
              <Trash2 size={16} strokeWidth={2} />
            </IconButton>
          </>
        }
      />

      <NoticeBars
        error={topError}
        dismissLabel={t("aiSummary.daily.actions.dismissError")}
        onDismissError={clearTopError}
        notice={topNotice}
      />

      {segments.length > 0 ? (
        <SegmentTimeline className={styles.card} data-part="card">
          {segments.map((seg, idx) => {
            const row = rows.get(idx);
            const isRunning = runningIdx === idx;
            const state: CardState = isRunning
              ? { kind: "running" }
              : !row
                ? { kind: "empty" }
                : row.status === "ok"
                  ? { kind: "ok", row }
                  : row.status === "skipped_no_screenshots" ||
                      row.status === "skipped_no_activity"
                    ? { kind: "skipped", row }
                    : { kind: "error", row };

            return (
              <SegmentRow
                key={idx}
                seg={seg}
                state={state}
                onRetry={() => void onRetrySegment(idx)}
                retryDisabled={generating}
                editing={editingIdx === idx}
                onEdit={() => setEditingIdx(idx)}
                onSaveEdit={(text) => onSaveEdit(idx, text)}
                onCancelEdit={() => setEditingIdx(null)}
              />
            );
          })}
        </SegmentTimeline>
      ) : null}
      <ConfirmDialog
        open={confirmingDelete}
        title={t("aiSummary.daily.actions.deleteConfirmTitle", { date })}
        message={t("aiSummary.daily.actions.deleteConfirmMessage")}
        variant="danger"
        onConfirm={async () => {
          setConfirmingDelete(false);
          try {
            await api.clearDaySummary(date, "daily");
            setRows(new Map());
            clearTopError();
          } catch (e) {
            setTopError(typeof e === "string" ? e : String(e));
          }
        }}
        onCancel={() => setConfirmingDelete(false)}
      />
      <ConfirmDialog
        open={pendingAfterDownload !== null}
        title={
          cloudConfigured
            ? t("aiSummary.daily.cloudOffer.title")
            : t("aiSummary.daily.runtimeMissing.title")
        }
        message={
          cloudConfigured
            ? t("aiSummary.daily.cloudOffer.message", {
                provider: settings?.ai.externalProvider || "cloud",
                model: settings?.ai.model || "",
              })
            : t("aiSummary.daily.runtimeMissing.message")
        }
        confirmLabel={
          cloudConfigured
            ? t("aiSummary.daily.cloudOffer.confirm")
            : t("aiSummary.daily.runtimeMissing.confirm")
        }
        cancelLabel={t("aiSummary.daily.runtimeMissing.cancel")}
        onConfirm={() => void onConfirmDownloadAi()}
        onCancel={() => setPendingAfterDownload(null)}
      />
    </>
  );
}

interface SegmentRowProps {
  seg: AiSegment;
  state: CardState;
  onRetry: () => void;
  /** 整轮生成中：重试和修改都不能点 */
  retryDisabled: boolean;
  /** 这一段正在修改 */
  editing: boolean;
  onEdit: () => void;
  onSaveEdit: (text: string) => Promise<void>;
  onCancelEdit: () => void;
}

/** 时间轴上的一段，加上生成中、未生成、重试、修改这些日报页才有的东西。 */
function SegmentRow(props: SegmentRowProps) {
  const { seg, state, onRetry, retryDisabled, editing, onEdit } = props;
  const { t } = useTranslation();
  return (
    <SegmentTimelineRow
      startHour={seg.startHour}
      endHour={seg.endHour}
      label={seg.label}
      color={resolveSegmentDotColor(seg)}
      hollow={state.kind === "empty" || state.kind === "skipped"}
      heading="h3"
      badge={
        state.kind === "running" ? (
          <span className={styles.runningChip}>
            <Loader2 size={12} className={ctl.spin} />
            {t("aiSummary.daily.card.badge.summarizing")}
          </span>
        ) : state.kind === "empty" ? (
          <span className={styles.mutedTag}>{t("aiSummary.daily.card.badge.empty")}</span>
        ) : state.kind === "ok" && !editing ? (
          <span className={styles.rowActions}>
            <IconButton
              label={t("aiSummary.edit.action")}
              small
              onClick={onEdit}
              disabled={retryDisabled}
              title={
                retryDisabled ? t("aiSummary.edit.runningTooltip") : t("aiSummary.edit.tooltip")
              }
            >
              <Pencil size={14} strokeWidth={2} />
            </IconButton>
            <IconButton
              label={t("aiSummary.daily.actions.retryTooltip")}
              small
              onClick={onRetry}
              disabled={retryDisabled}
              title={
                retryDisabled
                  ? t("aiSummary.daily.actions.retryDisabledTooltip")
                  : t("aiSummary.daily.actions.retryTooltip")
              }
            >
              <RotateCcw size={14} strokeWidth={2} />
            </IconButton>
          </span>
        ) : null
      }
    >
      <RowBody
        state={state}
        onRetry={onRetry}
        retryDisabled={retryDisabled}
        editing={editing}
        onSaveEdit={props.onSaveEdit}
        onCancelEdit={props.onCancelEdit}
      />
    </SegmentTimelineRow>
  );
}

function RowBody({
  state,
  onRetry,
  retryDisabled,
  editing,
  onSaveEdit,
  onCancelEdit,
}: Omit<SegmentRowProps, "seg" | "onEdit">) {
  const { t } = useTranslation();
  switch (state.kind) {
    case "empty":
      return null;
    case "running":
      return (
        <div className={ctl.skeleton} aria-hidden>
          <span />
          <span />
          <span />
        </div>
      );
    case "ok":
      return editing ? (
        <SummaryEditor initial={state.row.content} onSave={onSaveEdit} onCancel={onCancelEdit} />
      ) : (
        <MarkdownText text={state.row.content} className={tl.bodyText} />
      );
    case "skipped":
      return (
        <p className={tl.bodyMuted}>
          {state.row.status === "skipped_no_activity"
            ? t("aiSummary.daily.card.body.skippedNoActivity")
            : t("aiSummary.daily.card.body.skipped")}
        </p>
      );
    case "error":
      return (
        <div className={ctl.errorBox}>
          <AlertCircle size={14} strokeWidth={2} />
          <span className={ctl.errorText}>
            <strong>{t("aiSummary.daily.card.body.errorLabel")}</strong>
            {/* 段级错误也过 localizeAiError——[LLM_*]/[AI_*] 码换成本地化人话 */}
            {state.row.error
              ? localizeAiError(state.row.error)
              : t("aiSummary.daily.errors.unknown")}
          </span>
          <button
            type="button"
            className={ctl.textBtn}
            onClick={onRetry}
            disabled={retryDisabled}
            title={
              retryDisabled
                ? t("aiSummary.daily.actions.retryDisabledTooltip")
                : t("aiSummary.daily.actions.retryTooltip")
            }
          >
            {t("aiSummary.daily.actions.retry")}
          </button>
        </div>
      );
  }
}
