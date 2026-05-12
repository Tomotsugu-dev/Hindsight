import { useEffect, useMemo, useState } from "react";
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
  X,
} from "lucide-react";
import { useMouseGlow } from "../../../hooks/useMouseGlow";
import { useCategories } from "../../../state/categories";
import { logError } from "../../../lib/logger";
import {
  api,
  type QuickDaySummary,
  type QuickMonthSummary,
  type QuickUsageEntry,
  type QuickWeekSummary,
} from "../../../api/hindsight";
import {
  buildDayAnalysis,
  buildMonthAnalysis,
  buildWeekAnalysis,
  type QuickAnalysis,
} from "./quickAnalysis";
import styles from "./QuickSummaryView.module.css";

export type QuickScope = "day" | "week" | "month";

/** 快速模板总结的共用视图：纯 SQL 聚合 + 模板填空，瞬时返回，无 LLM 依赖。
 *
 *  内部自管：
 *  - 日期 / 周 / 月 导航（offset 状态本地保留，切 tab 不丢）
 *  - 数据加载（offset 变化即重拉）
 *  - 段落渲染（按 scope 走不同模板）
 *  - 导出 Markdown
 *
 *  外部需要：父 tab 渲染顶部的 SummaryModeToggle，本组件只渲染下方主体。 */
export function QuickSummaryView({ scope }: { scope: QuickScope }) {
  const { t } = useTranslation();
  const { categories } = useCategories();

  // 用 scope 作为 state 隔离 —— 切 tab 重新 mount，offset 自然重置
  const [offset, setOffset] = useState(0);
  const [day, setDay] = useState<QuickDaySummary | null>(null);
  const [week, setWeek] = useState<QuickWeekSummary | null>(null);
  const [month, setMonth] = useState<QuickMonthSummary | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);

  const { ref: prevBtnRef } = useMouseGlow<HTMLButtonElement>();
  const { ref: pillRef } = useMouseGlow<HTMLButtonElement>();
  const { ref: nextBtnRef } = useMouseGlow<HTMLButtonElement>();

  useEffect(() => {
    let cancelled = false;
    setLoading(true);
    setError(null);
    const fetch =
      scope === "day"
        ? api.getQuickDaySummary(offset)
        : scope === "week"
          ? api.getQuickWeekSummary(offset)
          : api.getQuickMonthSummary(offset);
    fetch
      .then((data) => {
        if (cancelled) return;
        if (scope === "day") setDay(data as QuickDaySummary);
        else if (scope === "week") setWeek(data as QuickWeekSummary);
        else setMonth(data as QuickMonthSummary);
      })
      .catch((e) => {
        if (cancelled) return;
        logError(`quick.${scope}.fetch`, e);
        setError(typeof e === "string" ? e : String(e));
      })
      .finally(() => {
        if (!cancelled) setLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, [scope, offset]);

  // 分类 id → 显示名的 map（'other' 走 i18n 兜底）
  const catName = useMemo(() => {
    const m = new Map<string, string>();
    for (const c of categories) m.set(c.id, c.name);
    return (id: string) => m.get(id) ?? t("aiSummary.quick.categoryOther");
  }, [categories, t]);

  // ───── 顶部导航 ─────
  const navLabel = useMemo(() => {
    if (scope === "day") {
      if (offset === 0) return t("aiSummary.quick.nav.day.today");
      if (offset === -1) return t("aiSummary.quick.nav.day.yesterday");
      return offsetToDateStr(offset);
    }
    if (scope === "week") {
      if (offset === 0) return t("aiSummary.quick.nav.week.thisWeek");
      if (offset === -1) return t("aiSummary.quick.nav.week.lastWeek");
      const { monday, sunday } = weekRangeFromOffset(offset);
      return `${toShortDate(monday)} ~ ${toShortDate(sunday)}`;
    }
    if (offset === 0) return t("aiSummary.quick.nav.month.thisMonth");
    if (offset === -1) return t("aiSummary.quick.nav.month.lastMonth");
    return monthLabelFromOffset(offset);
  }, [scope, offset, t]);

  const ariaPrev =
    scope === "day"
      ? t("aiSummary.quick.nav.day.prevAria")
      : scope === "week"
        ? t("aiSummary.quick.nav.week.prevAria")
        : t("aiSummary.quick.nav.month.prevAria");
  const ariaNext =
    scope === "day"
      ? t("aiSummary.quick.nav.day.nextAria")
      : scope === "week"
        ? t("aiSummary.quick.nav.week.nextAria")
        : t("aiSummary.quick.nav.month.nextAria");
  const backTooltip =
    scope === "day"
      ? t("aiSummary.quick.nav.day.todayBack")
      : scope === "week"
        ? t("aiSummary.quick.nav.week.thisWeekBack")
        : t("aiSummary.quick.nav.month.thisMonthBack");

  // ───── Markdown 导出 ─────
  const onExport = async () => {
    const body =
      scope === "day"
        ? buildDayMarkdown(day, t, catName)
        : scope === "week"
          ? buildWeekMarkdown(week, t, catName)
          : buildMonthMarkdown(month, t, catName);
    if (!body) {
      setError(t("aiSummary.quick.errors.empty"));
      return;
    }
    const filename =
      scope === "day"
        ? `hindsight-quick-day-${day?.date}.md`
        : scope === "week"
          ? `hindsight-quick-week-${week?.weekStart}.md`
          : `hindsight-quick-month-${month?.monthStart}.md`;

    let chosenPath: string | null = null;
    try {
      chosenPath = await save({
        title: t("aiSummary.quick.actions.exportMarkdown"),
        defaultPath: filename,
        filters: [{ name: "Markdown", extensions: ["md"] }],
      });
    } catch (e) {
      setError(typeof e === "string" ? e : String(e));
      return;
    }
    if (!chosenPath) return;
    try {
      await invoke("write_text_file", { path: chosenPath, content: body });
      setNotice(t("aiSummary.quick.toast.exported", { filename: chosenPath }));
      setTimeout(() => setNotice(null), 3500);
    } catch (e) {
      setError(typeof e === "string" ? e : String(e));
    }
  };

  const data: QuickDaySummary | QuickWeekSummary | QuickMonthSummary | null =
    scope === "day" ? day : scope === "week" ? week : month;
  const isEmpty = !loading && data != null && data.totalMinutes === 0;

  return (
    <>
      <p className={styles.subtitle}>
        {scope === "day"
          ? t("aiSummary.quick.subtitle.day")
          : scope === "week"
            ? t("aiSummary.quick.subtitle.week")
            : t("aiSummary.quick.subtitle.month")}
      </p>

      <header className={styles.header}>
        <div className={styles.nav}>
          <button
            ref={prevBtnRef}
            type="button"
            className={`${styles.navBtn} glow`}
            onClick={() => setOffset((v) => v - 1)}
            aria-label={ariaPrev}
          >
            <ChevronLeft size={14} strokeWidth={1.75} />
          </button>
          <button
            ref={pillRef}
            type="button"
            className={`${styles.pill} ${offset !== 0 ? styles.pillClickable : ""} glow`}
            onClick={() => setOffset(0)}
            disabled={offset === 0}
            title={offset === 0 ? undefined : backTooltip}
          >
            {navLabel}
          </button>
          <button
            ref={nextBtnRef}
            type="button"
            className={`${styles.navBtn} glow`}
            onClick={() => setOffset((v) => v + 1)}
            disabled={offset >= 0}
            aria-label={ariaNext}
          >
            <ChevronRight size={14} strokeWidth={1.75} />
          </button>
        </div>

        <button
          type="button"
          className={styles.exportBtn}
          onClick={() => void onExport()}
          disabled={loading || isEmpty || data == null}
          title={
            isEmpty
              ? t("aiSummary.quick.actions.exportEmptyTooltip")
              : t("aiSummary.quick.actions.exportTooltip")
          }
        >
          <Download size={14} strokeWidth={2} />
          {t("aiSummary.quick.actions.exportMarkdown")}
        </button>
      </header>

      {error ? (
        <div className={styles.errorBar}>
          <AlertTriangle size={14} strokeWidth={2.2} />
          <span>{error}</span>
          <button
            type="button"
            className={styles.errorClose}
            onClick={() => setError(null)}
            aria-label={t("aiSummary.quick.actions.dismissError")}
            title={t("aiSummary.quick.actions.dismissError")}
          >
            <X size={12} strokeWidth={2.4} />
          </button>
        </div>
      ) : null}

      {notice ? (
        <div className={styles.successBar}>
          <Check size={14} strokeWidth={2.4} />
          <span>{notice}</span>
        </div>
      ) : null}

      {loading ? (
        <div className={styles.loadingBox}>
          <Loader2 size={14} className={styles.spin} />
          <span>{t("aiSummary.quick.loading")}</span>
        </div>
      ) : isEmpty ? (
        <div className={styles.emptyBox}>
          <Clock size={14} strokeWidth={2} />
          <span>{t("aiSummary.quick.empty")}</span>
        </div>
      ) : scope === "day" && day ? (
        <DayView data={day} t={t} catName={catName} />
      ) : scope === "week" && week ? (
        <WeekView data={week} t={t} catName={catName} />
      ) : scope === "month" && month ? (
        <MonthView data={month} t={t} catName={catName} />
      ) : null}
    </>
  );
}

// ───────────────────────── DayView ─────────────────────────

type T = (key: string, opts?: Record<string, unknown>) => string;

function DayView({
  data,
  t,
  catName,
}: {
  data: QuickDaySummary;
  t: T;
  catName: (id: string) => string;
}) {
  const overview = t("aiSummary.quick.day.overview", {
    date: data.date,
    total: formatMinutes(t, data.totalMinutes),
    activeHours: data.activeHours,
    apps: data.topApps.length,
  });
  const peakLine =
    data.peakHour != null
      ? t("aiSummary.quick.day.peak", {
          hour: String(data.peakHour).padStart(2, "0"),
          nextHour: String(data.peakHour + 1).padStart(2, "0"),
          minutes: formatMinutes(t, data.peakHourMinutes),
        })
      : null;

  const visibleParts = data.dayParts.filter((p) => p.minutes > 0);
  const dayPartsLine =
    visibleParts.length > 0
      ? t("aiSummary.quick.day.distribution", {
          parts: visibleParts
            .map((p) =>
              t("aiSummary.quick.day.partTpl", {
                label: t(`aiSummary.quick.day.parts.${p.key}`),
                minutes: formatMinutes(t, p.minutes),
                percent: formatPercent(p.percent),
              }),
            )
            .join(t("aiSummary.quick.listSep")),
        })
      : null;

  const analysis = buildDayAnalysis(data, t, catName);

  return (
    <div className={styles.sections}>
      <Section title={t("aiSummary.quick.sections.overview")}>
        <p className={styles.para}>{overview}</p>
        {peakLine ? <p className={styles.para}>{peakLine}</p> : null}
        {dayPartsLine ? <p className={styles.para}>{dayPartsLine}</p> : null}
      </Section>
      <AnalysisSection analysis={analysis} t={t} />
      <UsageSection
        title={t("aiSummary.quick.sections.topApps")}
        entries={data.topApps}
        labelFor={(e) => e.key}
      />
      <UsageSection
        title={t("aiSummary.quick.sections.categories")}
        entries={data.categories}
        labelFor={(e) => catName(e.key)}
      />
    </div>
  );
}

// ───────────────────────── WeekView ─────────────────────────

function WeekView({
  data,
  t,
  catName,
}: {
  data: QuickWeekSummary;
  t: T;
  catName: (id: string) => string;
}) {
  const overview = t("aiSummary.quick.week.overview", {
    weekStart: data.weekStart,
    weekEnd: data.weekEnd,
    total: formatMinutes(t, data.totalMinutes),
    activeDays: data.activeDays,
    avg: formatMinutes(t, data.dailyAverageMinutes),
  });
  const peakLine = data.peakDay
    ? t("aiSummary.quick.week.peak", {
        date: data.peakDay.date,
        weekday: t(`aiSummary.quick.weekday.${data.peakDay.weekday}`),
        minutes: formatMinutes(t, data.peakDay.minutes),
      })
    : null;

  const analysis = buildWeekAnalysis(data, t, catName);

  return (
    <div className={styles.sections}>
      <Section title={t("aiSummary.quick.sections.overview")}>
        <p className={styles.para}>{overview}</p>
        {peakLine ? <p className={styles.para}>{peakLine}</p> : null}
      </Section>
      <AnalysisSection analysis={analysis} t={t} />
      <DailyBarSection
        title={t("aiSummary.quick.sections.dailySeries")}
        series={data.dailySeries}
        t={t}
      />
      <UsageSection
        title={t("aiSummary.quick.sections.topApps")}
        entries={data.topApps}
        labelFor={(e) => e.key}
      />
      <UsageSection
        title={t("aiSummary.quick.sections.categories")}
        entries={data.categories}
        labelFor={(e) => catName(e.key)}
      />
    </div>
  );
}

// ───────────────────────── MonthView ─────────────────────────

function MonthView({
  data,
  t,
  catName,
}: {
  data: QuickMonthSummary;
  t: T;
  catName: (id: string) => string;
}) {
  const overview = t("aiSummary.quick.month.overview", {
    monthStart: data.monthStart,
    monthEnd: data.monthEnd,
    totalDays: data.totalDays,
    total: formatMinutes(t, data.totalMinutes),
    activeDays: data.activeDays,
    avg: formatMinutes(t, data.dailyAverageMinutes),
  });
  const peakLine = data.peakDay
    ? t("aiSummary.quick.month.peak", {
        date: data.peakDay.date,
        weekday: t(`aiSummary.quick.weekday.${data.peakDay.weekday}`),
        minutes: formatMinutes(t, data.peakDay.minutes),
      })
    : null;
  const quietLine = data.quietDay
    ? t("aiSummary.quick.month.quiet", {
        date: data.quietDay.date,
        weekday: t(`aiSummary.quick.weekday.${data.quietDay.weekday}`),
        minutes: formatMinutes(t, data.quietDay.minutes),
      })
    : null;

  const analysis = buildMonthAnalysis(data, t, catName);

  return (
    <div className={styles.sections}>
      <Section title={t("aiSummary.quick.sections.overview")}>
        <p className={styles.para}>{overview}</p>
        {peakLine ? <p className={styles.para}>{peakLine}</p> : null}
        {quietLine ? <p className={styles.para}>{quietLine}</p> : null}
      </Section>
      <AnalysisSection analysis={analysis} t={t} />
      <DailyBarSection
        title={t("aiSummary.quick.sections.dailySeries")}
        series={data.dailySeries}
        t={t}
      />
      <UsageSection
        title={t("aiSummary.quick.sections.topApps")}
        entries={data.topApps}
        labelFor={(e) => e.key}
      />
      <UsageSection
        title={t("aiSummary.quick.sections.categories")}
        entries={data.categories}
        labelFor={(e) => catName(e.key)}
      />
    </div>
  );
}

// ───────────────────────── 子组件 ─────────────────────────

function Section({ title, children }: { title: string; children: React.ReactNode }) {
  return (
    <section className={styles.section}>
      <h3 className={styles.sectionTitle}>{title}</h3>
      <div className={styles.sectionBody}>{children}</div>
    </section>
  );
}

/** "伪 AI 分析"段落：工作内容概述 / 效率评估 / 改进建议 / 今日小结 + 底部 footnote。
 *  每个小节都按决策树挑了一句固定预设。整段为空时不渲染（极端兜底）。 */
function AnalysisSection({ analysis, t }: { analysis: QuickAnalysis; t: T }) {
  const items: { titleKey: string; body: string }[] = [];
  if (analysis.workOverview)
    items.push({
      titleKey: "aiSummary.quick.analysis.headings.workOverview",
      body: analysis.workOverview,
    });
  if (analysis.timePattern)
    items.push({
      titleKey: "aiSummary.quick.analysis.headings.timePattern",
      body: analysis.timePattern,
    });
  if (analysis.categoryBreakdown)
    items.push({
      titleKey: "aiSummary.quick.analysis.headings.categoryBreakdown",
      body: analysis.categoryBreakdown,
    });
  if (analysis.highlights)
    items.push({
      titleKey: "aiSummary.quick.analysis.headings.highlights",
      body: analysis.highlights,
    });
  if (analysis.focus)
    items.push({ titleKey: "aiSummary.quick.analysis.headings.focus", body: analysis.focus });
  if (analysis.efficiency)
    items.push({
      titleKey: "aiSummary.quick.analysis.headings.efficiency",
      body: analysis.efficiency,
    });
  if (analysis.suggestion)
    items.push({
      titleKey: "aiSummary.quick.analysis.headings.suggestion",
      body: analysis.suggestion,
    });
  if (analysis.summary)
    items.push({ titleKey: "aiSummary.quick.analysis.headings.summary", body: analysis.summary });

  if (items.length === 0) return null;

  return (
    <Section title={t("aiSummary.quick.analysis.title")}>
      <div className={styles.analysisGroups}>
        {items.map((it, i) => (
          <div key={i} className={styles.analysisItem}>
            <h4 className={styles.analysisItemTitle}>{t(it.titleKey)}</h4>
            <p className={styles.analysisItemBody}>{it.body}</p>
          </div>
        ))}
      </div>
      <p className={styles.analysisFootnote}>{t("aiSummary.quick.analysis.footnote")}</p>
    </Section>
  );
}

/** 通用占比列表：top apps 或 categories 共用。`labelFor` 让调用方决定 key → 显示名的映射。 */
function UsageSection({
  title,
  entries,
  labelFor,
}: {
  title: string;
  entries: QuickUsageEntry[];
  labelFor: (e: QuickUsageEntry) => string;
}) {
  const { t } = useTranslation();
  if (entries.length === 0) {
    return (
      <Section title={title}>
        <p className={styles.muted}>{t("aiSummary.quick.empty")}</p>
      </Section>
    );
  }
  return (
    <Section title={title}>
      <ul className={styles.usageList}>
        {entries.map((e, i) => (
          <li key={`${e.key}-${i}`} className={styles.usageRow}>
            <span className={styles.usageLabel}>{labelFor(e)}</span>
            <div className={styles.usageBarTrack}>
              <div
                className={styles.usageBarFill}
                style={{ width: `${Math.max(2, Math.round(e.percent * 100))}%` }}
              />
            </div>
            <span className={styles.usageValue}>
              {formatMinutes(t, e.minutes)} · {formatPercent(e.percent)}
            </span>
          </li>
        ))}
      </ul>
    </Section>
  );
}

/** 周 / 月报里逐日时长 mini bar chart。最长那天填满，其它按比例。 */
function DailyBarSection({
  title,
  series,
  t,
}: {
  title: string;
  series: { date: string; minutes: number }[];
  t: T;
}) {
  const max = series.reduce((m, p) => (p.minutes > m ? p.minutes : m), 0);
  return (
    <Section title={title}>
      <ul className={styles.dailyList}>
        {series.map((p) => (
          <li key={p.date} className={styles.dailyRow}>
            <span className={styles.dailyDate}>{toShortDate(p.date)}</span>
            <div className={styles.dailyBarTrack}>
              <div
                className={styles.dailyBarFill}
                style={{
                  width: max > 0 ? `${Math.max(2, Math.round((p.minutes / max) * 100))}%` : "0%",
                  opacity: p.minutes > 0 ? 1 : 0,
                }}
              />
            </div>
            <span className={styles.dailyValue}>{formatMinutes(t, p.minutes)}</span>
          </li>
        ))}
      </ul>
    </Section>
  );
}

// ───────────────────────── 工具函数 ─────────────────────────

function offsetToDateStr(off: number): string {
  const d = new Date();
  d.setDate(d.getDate() + off);
  const y = d.getFullYear();
  const m = String(d.getMonth() + 1).padStart(2, "0");
  const day = String(d.getDate()).padStart(2, "0");
  return `${y}-${m}-${day}`;
}

function toShortDate(s: string): string {
  return s.length === 10 ? s.slice(5) : s;
}

function weekRangeFromOffset(off: number): { monday: string; sunday: string } {
  const today = new Date();
  const dow = (today.getDay() + 6) % 7;
  const monday = new Date(today);
  monday.setDate(today.getDate() - dow + off * 7);
  const sunday = new Date(monday);
  sunday.setDate(monday.getDate() + 6);
  const fmt = (d: Date) => {
    const y = d.getFullYear();
    const m = String(d.getMonth() + 1).padStart(2, "0");
    const day = String(d.getDate()).padStart(2, "0");
    return `${y}-${m}-${day}`;
  };
  return { monday: fmt(monday), sunday: fmt(sunday) };
}

function monthLabelFromOffset(off: number): string {
  const today = new Date();
  const target = new Date(today.getFullYear(), today.getMonth() + off, 1);
  return `${target.getFullYear()}-${String(target.getMonth() + 1).padStart(2, "0")}`;
}

function formatMinutes(t: T, m: number): string {
  if (m < 60) return t("aiSummary.quick.duration.minutes", { count: m });
  const h = Math.floor(m / 60);
  const mm = m % 60;
  if (mm === 0) return t("aiSummary.quick.duration.hours", { count: h });
  return t("aiSummary.quick.duration.hoursMinutes", { hours: h, minutes: mm });
}

function formatPercent(p: number): string {
  return `${(p * 100).toFixed(p < 0.1 ? 1 : 0)}%`;
}

// ───────────────────────── Markdown 导出 ─────────────────────────

function buildDayMarkdown(
  data: QuickDaySummary | null,
  t: T,
  catName: (id: string) => string,
): string | null {
  if (!data) return null;
  const lines: string[] = ["---"];
  lines.push(`title: ${t("aiSummary.quick.export.day.frontmatterTitle", { date: data.date })}`);
  lines.push(`date: ${data.date}`);
  lines.push(`mode: quick`);
  lines.push("---", "");
  lines.push(`# ${t("aiSummary.quick.export.day.heading", { date: data.date })}`, "");
  lines.push(
    t("aiSummary.quick.day.overview", {
      date: data.date,
      total: formatMinutes(t, data.totalMinutes),
      activeHours: data.activeHours,
      apps: data.topApps.length,
    }),
    "",
  );
  pushAnalysisSection(lines, t, buildDayAnalysis(data, t, catName));
  if (data.peakHour != null) {
    lines.push(
      t("aiSummary.quick.day.peak", {
        hour: String(data.peakHour).padStart(2, "0"),
        nextHour: String(data.peakHour + 1).padStart(2, "0"),
        minutes: formatMinutes(t, data.peakHourMinutes),
      }),
      "",
    );
  }
  const parts = data.dayParts.filter((p) => p.minutes > 0);
  if (parts.length > 0) {
    lines.push(
      t("aiSummary.quick.day.distribution", {
        parts: parts
          .map((p) =>
            t("aiSummary.quick.day.partTpl", {
              label: t(`aiSummary.quick.day.parts.${p.key}`),
              minutes: formatMinutes(t, p.minutes),
              percent: formatPercent(p.percent),
            }),
          )
          .join(t("aiSummary.quick.listSep")),
      }),
      "",
    );
  }
  pushUsageSection(lines, t, t("aiSummary.quick.sections.topApps"), data.topApps, (e) => e.key);
  pushUsageSection(lines, t, t("aiSummary.quick.sections.categories"), data.categories, (e) =>
    catName(e.key),
  );
  return lines.join("\n");
}

function buildWeekMarkdown(
  data: QuickWeekSummary | null,
  t: T,
  catName: (id: string) => string,
): string | null {
  if (!data) return null;
  const lines: string[] = ["---"];
  lines.push(
    `title: ${t("aiSummary.quick.export.week.frontmatterTitle", {
      weekStart: data.weekStart,
      weekEnd: data.weekEnd,
    })}`,
  );
  lines.push(`week_start: ${data.weekStart}`);
  lines.push(`week_end: ${data.weekEnd}`);
  lines.push(`mode: quick`);
  lines.push("---", "");
  lines.push(
    `# ${t("aiSummary.quick.export.week.heading", {
      weekStart: data.weekStart,
      weekEnd: data.weekEnd,
    })}`,
    "",
  );
  lines.push(
    t("aiSummary.quick.week.overview", {
      weekStart: data.weekStart,
      weekEnd: data.weekEnd,
      total: formatMinutes(t, data.totalMinutes),
      activeDays: data.activeDays,
      avg: formatMinutes(t, data.dailyAverageMinutes),
    }),
    "",
  );
  pushAnalysisSection(lines, t, buildWeekAnalysis(data, t, catName));
  if (data.peakDay) {
    lines.push(
      t("aiSummary.quick.week.peak", {
        date: data.peakDay.date,
        weekday: t(`aiSummary.quick.weekday.${data.peakDay.weekday}`),
        minutes: formatMinutes(t, data.peakDay.minutes),
      }),
      "",
    );
  }
  pushDailySeriesSection(lines, t, data.dailySeries);
  pushUsageSection(lines, t, t("aiSummary.quick.sections.topApps"), data.topApps, (e) => e.key);
  pushUsageSection(lines, t, t("aiSummary.quick.sections.categories"), data.categories, (e) =>
    catName(e.key),
  );
  return lines.join("\n");
}

function buildMonthMarkdown(
  data: QuickMonthSummary | null,
  t: T,
  catName: (id: string) => string,
): string | null {
  if (!data) return null;
  const lines: string[] = ["---"];
  lines.push(
    `title: ${t("aiSummary.quick.export.month.frontmatterTitle", {
      monthStart: data.monthStart,
      monthEnd: data.monthEnd,
    })}`,
  );
  lines.push(`month_start: ${data.monthStart}`);
  lines.push(`month_end: ${data.monthEnd}`);
  lines.push(`mode: quick`);
  lines.push("---", "");
  lines.push(
    `# ${t("aiSummary.quick.export.month.heading", {
      monthStart: data.monthStart,
      monthEnd: data.monthEnd,
    })}`,
    "",
  );
  lines.push(
    t("aiSummary.quick.month.overview", {
      monthStart: data.monthStart,
      monthEnd: data.monthEnd,
      totalDays: data.totalDays,
      total: formatMinutes(t, data.totalMinutes),
      activeDays: data.activeDays,
      avg: formatMinutes(t, data.dailyAverageMinutes),
    }),
    "",
  );
  pushAnalysisSection(lines, t, buildMonthAnalysis(data, t, catName));
  if (data.peakDay) {
    lines.push(
      t("aiSummary.quick.month.peak", {
        date: data.peakDay.date,
        weekday: t(`aiSummary.quick.weekday.${data.peakDay.weekday}`),
        minutes: formatMinutes(t, data.peakDay.minutes),
      }),
      "",
    );
  }
  if (data.quietDay) {
    lines.push(
      t("aiSummary.quick.month.quiet", {
        date: data.quietDay.date,
        weekday: t(`aiSummary.quick.weekday.${data.quietDay.weekday}`),
        minutes: formatMinutes(t, data.quietDay.minutes),
      }),
      "",
    );
  }
  pushDailySeriesSection(lines, t, data.dailySeries);
  pushUsageSection(lines, t, t("aiSummary.quick.sections.topApps"), data.topApps, (e) => e.key);
  pushUsageSection(lines, t, t("aiSummary.quick.sections.categories"), data.categories, (e) =>
    catName(e.key),
  );
  return lines.join("\n");
}

function pushAnalysisSection(lines: string[], t: T, analysis: QuickAnalysis) {
  const items: { titleKey: string; body: string }[] = [];
  if (analysis.workOverview)
    items.push({
      titleKey: "aiSummary.quick.analysis.headings.workOverview",
      body: analysis.workOverview,
    });
  if (analysis.timePattern)
    items.push({
      titleKey: "aiSummary.quick.analysis.headings.timePattern",
      body: analysis.timePattern,
    });
  if (analysis.categoryBreakdown)
    items.push({
      titleKey: "aiSummary.quick.analysis.headings.categoryBreakdown",
      body: analysis.categoryBreakdown,
    });
  if (analysis.highlights)
    items.push({
      titleKey: "aiSummary.quick.analysis.headings.highlights",
      body: analysis.highlights,
    });
  if (analysis.focus)
    items.push({ titleKey: "aiSummary.quick.analysis.headings.focus", body: analysis.focus });
  if (analysis.efficiency)
    items.push({
      titleKey: "aiSummary.quick.analysis.headings.efficiency",
      body: analysis.efficiency,
    });
  if (analysis.suggestion)
    items.push({
      titleKey: "aiSummary.quick.analysis.headings.suggestion",
      body: analysis.suggestion,
    });
  if (analysis.summary)
    items.push({ titleKey: "aiSummary.quick.analysis.headings.summary", body: analysis.summary });
  if (items.length === 0) return;

  lines.push(`## ${t("aiSummary.quick.analysis.title")}`, "");
  items.forEach((it) => {
    lines.push(`### ${t(it.titleKey)}`, "", it.body, "");
  });
  lines.push(`> ${t("aiSummary.quick.analysis.footnote")}`, "");
}

function pushUsageSection(
  lines: string[],
  t: T,
  title: string,
  entries: QuickUsageEntry[],
  label: (e: QuickUsageEntry) => string,
) {
  if (entries.length === 0) return;
  lines.push(`## ${title}`, "");
  entries.forEach((e, i) => {
    lines.push(
      `${i + 1}. **${label(e)}** — ${formatMinutes(t, e.minutes)} · ${formatPercent(e.percent)}`,
    );
  });
  lines.push("");
}

function pushDailySeriesSection(
  lines: string[],
  t: T,
  series: { date: string; minutes: number }[],
) {
  if (series.length === 0) return;
  lines.push(`## ${t("aiSummary.quick.sections.dailySeries")}`, "");
  series.forEach((p) => {
    lines.push(`- ${p.date} — ${formatMinutes(t, p.minutes)}`);
  });
  lines.push("");
}
