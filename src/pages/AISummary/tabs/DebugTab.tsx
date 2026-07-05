import { useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { listen } from "@tauri-apps/api/event";
import { save } from "@tauri-apps/plugin-dialog";
import { useMouseGlow } from "../../../hooks/useMouseGlow";
import {
  AlertTriangle,
  ChevronLeft,
  ChevronRight,
  Clock,
  Download,
  Info,
  Loader2,
  MessageSquareText,
  Newspaper,
  Play,
  Square,
  Trash2,
} from "lucide-react";
import {
  api,
  SUMMARY_CLOUD_SENTINEL,
  SUMMARY_PROGRESS_EVENT,
  type AiOverrides,
  type EngineStatus,
  type SegmentSummaryRow,
  type SummaryProgress,
} from "../../../api/hindsight";
import { useSettings } from "../../../state/settings";
import { useDebugState } from "../DebugStateContext";
import { resolveSegmentDotColor } from "../../../utils/segmentColor";
import { SimplePicker } from "../../../components/SimplePicker/SimplePicker";
import { Row } from "../../../components/FormLayout/Row";
import { Section } from "../../../components/FormLayout/Section";
import { ConfirmDialog } from "../../../components/ConfirmDialog/ConfirmDialog";
import { DEFAULT_SYSTEM_PROMPTS } from "../../../lib/aiPrompts";
import { logError, logWarn } from "../../../lib/logger";
import {
  type LogEntry,
  type DebugScope,
  buildScopeOptions,
  LOG_RING_SIZE,
  anchorDateStr,
  offsetLabel,
  nowHms,
  fmtPhaseBody,
} from "./debugTabHelpers";
import { EngineBar } from "./EngineBar";
import { MarkdownText } from "../../../components/MarkdownText/MarkdownText";
import styles from "./DebugTab.module.css";

/**
 * 调试 tab：跑一天 debug 总结 + 看结果 + 引擎日志。
 *  - 引擎状态条（getEngineStatus）
 *  - 日期导航 + 开始 / 停止（复用 generateDaySummary / cancelDaySummary，source="debug"）
 *  - 实时事件流 log（listen 全部 phase）
 *  - 段总结结果（getDaySummary + listen segment_done）+ 导出 Markdown
 *  - 引擎启动日志（llama-server stderr / stdout）
 */
export default function DebugTab() {
  const { t } = useTranslation();
  const { settings } = useSettings();
  const segments = settings?.ai.segments ?? [];
  const activeMain = settings?.ai.activeMain ?? "";
  // hasModel 跟 DailyTab + 后端 summary_runner check 对齐：段总结走
  // 选定云端（sentinel + externalEnabled）或本地 summary 或 activeMain fallback
  const rawSummaryMain = settings?.ai.summaryMain ?? "";
  const externalEnabled = settings?.ai.externalEnabled ?? false;
  const cloudRoute = externalEnabled && rawSummaryMain === SUMMARY_CLOUD_SENTINEL;
  const localSummaryAvailable =
    (rawSummaryMain !== "" && rawSummaryMain !== SUMMARY_CLOUD_SENTINEL) ||
    activeMain !== "";
  const hasModel = cloudRoute || localSummaryAvailable;

  // Picker 选项随语言变化而重建——i18next 切语言会让 t 引用变更，触发 useMemo 重算
  const SCOPE_OPTIONS = useMemo(() => buildScopeOptions(t), [t]);

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
  /** 成功提示（绿色 successBar）：导出 markdown 落盘后短暂显示，3.5s 自清。 */
  const [topNotice, setTopNotice] = useState<string | null>(null);

  // 调试参数 state 来自 Context，跟 DebugSettingsTab 共享同一份。
  const {
    debugExcluded,
    debugSummaryBatchSize,
    debugSummaryParallelSlots,
    debugSummaryCtxSize,
    debugSysPrompt,
    setDebugSysPrompt,
    debugExternalEnabled,
  } = useDebugState();

  const [engine, setEngine] = useState<EngineStatus | null>(null);
  const [summaries, setSummaries] = useState<SegmentSummaryRow[]>([]);
  const [logs, setLogs] = useState<LogEntry[]>([]);
  /** llama-server 启动日志（GPU 加载情况、cuBLAS init 等）；点刷新拉一次 */
  const [engineLogs, setEngineLogs] = useState<string[]>([]);
  const [engineLogsBusy, setEngineLogsBusy] = useState(false);
  const [engineLogsCopied, setEngineLogsCopied] = useState(false);
  // 清段总结确认弹框
  const [confirmingClear, setConfirmingClear] = useState(false);

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

  const copyEngineLogs = async () => {
    if (engineLogs.length === 0) return;
    try {
      await navigator.clipboard.writeText(engineLogs.join("\n"));
      setEngineLogsCopied(true);
      setTimeout(() => setEngineLogsCopied(false), 1500);
    } catch (e) {
      logWarn("debug.copyEngineLogs", e);
    }
  };

  // 锚定日期：daily=当天，weekly=该周一，monthly=该月 1 号；
  // 周报 / 月报命令未来传这个值。daily 之外的 scope 现在 onStart 会被拦掉，
  // 所以 anchor 暂时只用于 listen 的 date 比对（避免日报跑动时事件被误算成周报的）。
  const date = useMemo(() => anchorDateStr(scope, dayOffset), [scope, dayOffset]);

  // 进页 / 切日期：拉引擎状态 + 段总结
  useEffect(() => {
    let cancelled = false;
    setSummaries([]);
    setLogs([]);
    setEnginePhase(null);
    setTopError(null);

    void Promise.all([
      api.getEngineStatus().catch((e) => {
        logError("debug.getEngineStatus", e);
        return null;
      }),
      api.getDaySummary(date, "debug").catch(() => [] as SegmentSummaryRow[]),
    ]).then(([eng, sums]) => {
      if (cancelled) return;
      setEngine(eng);
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


  /** 跑一天 debug 总结（完整流程）。 */
  const onStart = async () => {
    if (scope !== "daily") {
      setTopError(t("aiSummary.debug.errors.scopePending", { type: scopeName(scope) }));
      return;
    }
    if (!hasModel) {
      setTopError(t("aiSummary.debug.errors.noModel"));
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
      const overrides: AiOverrides = {
        excludedCategories: debugExcluded,
        systemPrompt:
          debugSysPrompt.trim() === sysPromptDefault.trim() ? "" : debugSysPrompt,
        // 引擎参数——null / 1 = 不传，让后端 fallback 到 settings.ai 默认；
        // 非默认值才打包到对应的 summary* 字段，触发 stop+start with overrides
        ...(debugSummaryBatchSize != null ? { summaryBatchSize: debugSummaryBatchSize } : {}),
        ...(debugSummaryParallelSlots > 1 ? { summaryParallelSlots: debugSummaryParallelSlots } : {}),
        ...(debugSummaryCtxSize != null ? { summaryCtxSize: debugSummaryCtxSize } : {}),
        // 跟 settings 全局值不同才传——一致的话留 undefined 让后端走 settings.ai.externalEnabled
        ...(debugExternalEnabled !== (settings?.ai.externalEnabled ?? false)
          ? { externalEnabled: debugExternalEnabled }
          : {}),
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
      logWarn("debug.cancel", e);
    }
  };

  /** 把 markdown 文本写到用户选的路径。
   *
   *  Tauri webview 不可靠支持浏览器原生 `<a download>`——点了静默失败 / 用户找不到文件去哪。
   *  改用 Tauri save dialog 让用户主动选位置 + 后端 std::fs 写文件 + 顶部绿色成功条
   *  显示落盘绝对路径（3.5s 自清，跟 Daily 同款 UX）。
   *
   *  失败时：用户取消 dialog → 静默 return（不报错）；后端写失败 → setTopError。 */
  const downloadMarkdown = async (md: string, filename: string) => {
    let chosenPath: string | null = null;
    try {
      chosenPath = await save({
        title: t("aiSummary.debug.actions.exportSaveTitle"),
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
      setTopNotice(
        t("aiSummary.debug.actions.exportSavedAt", { path: chosenPath }),
      );
      // successBar 3.5s 自清——给用户看清楚但不挡视线
      setTimeout(() => setTopNotice(null), 3500);
    } catch (e) {
      setTopError(
        t("aiSummary.debug.actions.exportFailed", {
          err: typeof e === "string" ? e : String(e),
        }),
      );
    }
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
      } else if (row.status === "skipped_no_activity") {
        lines.push("_skipped (no activity in this segment)_", "");
      } else {
        lines.push("_error_", "");
        if (row.error) lines.push(`> ${row.error.replace(/\n/g, "\n> ")}`, "");
      }
      if (i < sorted.length - 1) lines.push("---", "");
    });

    void downloadMarkdown(lines.join("\n"), `hindsight-debug-summaries-${date}.md`);
  };

  // 周报 / 月报后端没实现，总结按 scope 切：非 daily 时清空显示
  const visibleSummaries = scope === "daily" ? summaries : [];

  return (
    <>
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
            <button
              type="button"
              className={styles.infoIconWrap}
              title={t("aiSummary.debug.actions.startInfo")}
              aria-label={t("aiSummary.debug.actions.startInfoAria")}
            >
              <Info size={13} strokeWidth={1.85} />
            </button>
          </>
        )}
      </div>


      {/* —— 引擎状态条 —— */}
      <EngineBar engine={engine} />

      {/* —— 错误条 / 成功条 / 冷启动提示 —— */}
      {topError ? (
        <div className={styles.errorBar}>
          <AlertTriangle size={14} strokeWidth={2.2} />
          <span>{topError}</span>
        </div>
      ) : null}
      {topNotice ? (
        <div className={styles.successBar}>
          <span>{topNotice}</span>
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

      {/* —— 时间段总结提示词：独立 Section box；textarea 默认收起，hover 该卡片展开 —— */}
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

      {/* —— 段总结结果：Section 常开 + 滚动。headerAction：「删除」(只清段总结) + 导出。 */}
      <Section
        title={t("aiSummary.debug.segments.title")}
        icon={Newspaper}
        headerAction={
          <>
            <button
              type="button"
              className={styles.deleteBtn}
              onClick={() => {
                if (generating) return;
                setConfirmingClear(true);
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
              // 段标签＝色点 + 中性文字（跟 DailyTab 同款）：颜色只出现在圆点上。
              // settings 未加载到该段时圆点退回中性灰。
              const seg = segments[s.segmentIdx];
              const dotColor = seg ? resolveSegmentDotColor(seg) : "#cbd5e1";
              // 状态徽章：ok 绿 / error 红 / skipped 灰
              const isSkipped =
                s.status === "skipped_no_screenshots" ||
                s.status === "skipped_no_activity";
              const statusClass =
                s.status === "ok"
                  ? styles.summaryStatusOk
                  : isSkipped
                    ? styles.summaryStatusSkipped
                    : styles.summaryStatusError;
              const statusText = s.status === "ok" ? "ok" : isSkipped ? "skipped" : "error";
              return (
                <div key={s.segmentIdx} className={styles.summaryBox}>
                  <div className={styles.summaryHead}>
                    <span
                      className={styles.summaryChip}
                      title={t("aiSummary.debug.segments.chipTitle", { idx: s.segmentIdx })}
                    >
                      <span
                        className={styles.segDot}
                        style={{ background: dotColor }}
                        aria-hidden
                      />
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
                  {s.content ? (
                    <MarkdownText text={s.content} className={styles.summaryText} />
                  ) : (
                    <div className={styles.summaryText}>
                      {s.status === "skipped_no_screenshots"
                        ? t("aiSummary.debug.segments.skippedFallback")
                        : s.status === "skipped_no_activity"
                          ? t("aiSummary.debug.segments.skippedNoActivityFallback")
                          : s.error || t("aiSummary.debug.segments.emptyFallback")}
                    </div>
                  )}
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
          <button
            type="button"
            className={styles.engineLogsRefreshBtn}
            onClick={() => void copyEngineLogs()}
            disabled={engineLogs.length === 0}
            title={t("aiSummary.debug.engineLogs.copyTooltip")}
          >
            {engineLogsCopied
              ? t("aiSummary.debug.engineLogs.copied")
              : t("aiSummary.debug.engineLogs.copy")}
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
    <ConfirmDialog
      open={confirmingClear}
      title={t("aiSummary.debug.actions.clearSummariesConfirmTitle", { date })}
      message={t("aiSummary.debug.actions.clearSummariesConfirmMessage")}
      variant="danger"
      onConfirm={async () => {
        setConfirmingClear(false);
        try {
          await api.clearDaySegmentSummaries(date, "debug");
          setSummaries([]);
        } catch (e) {
          setTopError(typeof e === "string" ? e : String(e));
        }
      }}
      onCancel={() => setConfirmingClear(false)}
    />
    </>
  );
}
