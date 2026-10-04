import { useCallback, useMemo, useState, type CSSProperties } from "react";
import { createPortal } from "react-dom";
import { useOutletContext } from "react-router-dom";
import { useTranslation } from "react-i18next";
import { ChevronRight, X } from "lucide-react";
import { EmptyHint } from "../../components/EmptyHint/EmptyHint";
import { ScrollBox } from "../../components/ScrollBox/ScrollBox";
import {
  RankedList,
  type RankedItem,
} from "../../components/RankedList/RankedList";
import { useDeviceFilter } from "../../state/deviceFilter";
import { usePeriodRankings } from "../../hooks/usePeriodRankings";
import {
  catMinutesFromSegments,
  useSuperCategoryBreakdown,
} from "../../hooks/useSuperCategoryBreakdown";
import { resolveCategoryIcon } from "../../config/categoryIcons";
import { useDurationFormatter } from "../../utils/duration";
import type { AppUsage, DaySummaryDto } from "../../api/hindsight";
import { AppDetailCard, type HistoryApp } from "./AppDetailCard";
import { Heatmap } from "./Heatmap";
import { clipRange, dayKey, parseDayKey, type DayRange } from "./heatmapGrid";
import {
  HistoryChartCard,
  type ChartTab,
  type PeriodNav,
} from "./HistoryChartCard";
import type { HistoryOutletContext } from "./HistoryPage";
import { MonthCalendar } from "./MonthCalendar";
import {
  useAppRangeDetail,
  useEarliestDate,
  useRangeApps,
  useRangeDays,
} from "./useHistoryData";
import styles from "./HistoryPage.module.css";

const NO_DAYS: DaySummaryDto[] = [];
const NO_APPS: AppUsage[] = [];

/** 没选大类时图表用的颜色（同 --color-accent 亮色） */
const ACCENT = "#6c5ce7";

/**
 * 全部历史的「统计」标签：上面是图表卡片（年表 ｜ 月表），下面左边大类、右边应用。
 *
 * 下面的列表跟着现在看的范围走：选中的那一天，或者热力图上拖出来的那段，或者这一年、这个月。
 * 点大类只筛选。点应用：上面的图换成这个应用的；下面三张卡（大类、应用、窗口详情）
 * 排在一条轨道上整体左移一格，大类移出去，窗口详情进来。
 */
export default function StatsTab() {
  const { t, i18n } = useTranslation();
  const { selectedDeviceId } = useDeviceFilter();
  const fmtHM = useDurationFormatter();
  const { metaSlot } = useOutletContext<HistoryOutletContext>();

  const earliest = useEarliestDate();
  const today = dayKey(new Date());
  const from = earliest ?? null;
  const daysLoad = useRangeDays(from, today, selectedDeviceId);
  // 换设备时先拿上一份顶着，等新的到了再换，页面不闪
  const days = daysLoad.data ?? daysLoad.latest ?? NO_DAYS;

  const now = new Date();
  const thisYear = now.getFullYear();
  const thisMonthIdx = thisYear * 12 + now.getMonth();
  const firstDate = from === null ? now : parseDayKey(from);
  const minYear = firstDate.getFullYear();
  const minMonthIdx = minYear * 12 + firstDate.getMonth();

  const [tab, setTab] = useState<ChartTab>("year");
  const [year, setYear] = useState(thisYear);
  /** 年 × 12 + 月，前后翻月只要加减 1 */
  const [monthIdx, setMonthIdx] = useState(thisMonthIdx);
  const [zoom, setZoom] = useState<DayRange | null>(null);
  const [day, setDay] = useState<string | null>(null);
  const [superId, setSuperId] = useState<string | null>(null);
  // app 是窗口详情卡上的应用；open 决定轨道是否左移。关的时候先滑回去，
  // 滑完（transitionend）再清 app，免得卡片在滑出途中变空
  const [app, setApp] = useState<HistoryApp | null>(null);
  const [open, setOpen] = useState(false);
  const activeApp = open ? app : null;

  // 换标签、换年、换月时，放大的那段和选中的天都不再有意义
  const changeTab = (next: ChartTab) => {
    setTab(next);
    setZoom(null);
    setDay(null);
  };
  const changeYear = (next: number) => {
    setYear(next);
    setZoom(null);
    setDay(null);
  };
  const changeMonth = (next: number) => {
    setMonthIdx(next);
    setZoom(null);
    setDay(null);
  };
  // Heatmap 的 effect 依赖这三个回调，保持引用不变，免得拖动中途重挂监听
  const toggleDay = useCallback(
    (key: string) => setDay((prev) => (prev === key ? null : key)),
    [],
  );
  const zoomTo = useCallback((range: DayRange) => {
    setZoom(range);
    setDay(null);
  }, []);
  const resetZoom = useCallback(() => setZoom(null), []);
  const closeApp = () => setOpen(false);
  // 再点已经打开的那个应用就关上；点别的应用直接换详情
  const clickApp = (item: RankedItem) => {
    if (activeApp?.groupId === item.id) {
      closeApp();
      return;
    }
    setApp({
      name: item.name,
      groupId: item.id,
      iconProcess: item.iconProcess ?? item.id,
      categoryLabel: item.subtitle,
      color: item.color,
    });
    setOpen(true);
  };

  // —— 范围：这一年或这个月 → 拖出来的那段 → 选中的那天，后面的盖过前面的 ——
  const mYear = Math.floor(monthIdx / 12);
  const mMonth = monthIdx % 12;
  const yearRange = useMemo(() => ({ from: `${year}-01-01`, to: `${year}-12-31` }), [year]);
  const domain = useMemo<DayRange | null>(() => {
    if (from === null) return null;
    const bounds = { from, to: today };
    const period =
      tab === "year"
        ? yearRange
        : { from: dayKey(new Date(mYear, mMonth, 1)), to: dayKey(new Date(mYear, mMonth + 1, 0)) };
    return clipRange(period, bounds) ?? bounds;
  }, [from, today, tab, yearRange, mYear, mMonth]);
  const view = zoom ?? domain;
  const scopeFrom = day ?? view?.from ?? null;
  const scopeTo = day ?? view?.to ?? null;

  // —— 大类：左边列表跟着范围走；筛选用的分类集合按全部历史算，换范围不丢 ——
  const scopedDays = useMemo(
    () =>
      scopeFrom === null || scopeTo === null
        ? NO_DAYS
        : days.filter((d) => d.date >= scopeFrom && d.date <= scopeTo),
    [days, scopeFrom, scopeTo],
  );
  const allCatMinutes = useMemo(() => catMinutesFromSegments(days), [days]);
  const scopedCatMinutes = useMemo(() => catMinutesFromSegments(scopedDays), [scopedDays]);
  const allBreakdown = useSuperCategoryBreakdown(allCatMinutes);
  const scopedBreakdown = useSuperCategoryBreakdown(scopedCatMinutes);
  const selectedSuper =
    superId === null ? null : (allBreakdown.slices.find((s) => s.id === superId) ?? null);
  const superCatIds = useMemo(
    () => (selectedSuper ? new Set(selectedSuper.cats.map((c) => c.id)) : null),
    [selectedSuper],
  );

  const superItems = useMemo<RankedItem[]>(
    () =>
      scopedBreakdown.slices.map((s) => {
        const Icon = resolveCategoryIcon(s.icon);
        return {
          id: s.id,
          name: s.name,
          color: s.color,
          minutes: s.minutes,
          leading: (
            <span
              className={styles.superIcon}
              style={{ "--row-color": s.color } as CSSProperties}
              aria-hidden
            >
              <Icon size={14} strokeWidth={2} />
            </span>
          ),
        };
      }),
    [scopedBreakdown.slices],
  );

  // —— 应用：拉这个范围的；选了大类就只留这个大类下的 ——
  const appsLoad = useRangeApps(scopeFrom, scopeTo, selectedDeviceId);
  const scopedApps = appsLoad.data ?? appsLoad.latest ?? NO_APPS;
  const shownApps = useMemo(
    () =>
      superCatIds ? scopedApps.filter((a) => superCatIds.has(a.categoryId)) : scopedApps,
    [scopedApps, superCatIds],
  );
  const { appRanks } = usePeriodRankings(scopedDays, shownApps);
  const appsMinutes = shownApps.reduce((sum, a) => sum + a.minutes, 0);

  // —— 图表的每日秒数：总览按大类筛，应用页用这个应用的 ——
  const overviewSecs = useMemo(() => {
    const m = new Map<string, number>();
    for (const d of days) {
      let secs = 0;
      for (const seg of d.segments) {
        if (!superCatIds || superCatIds.has(seg.categoryId)) secs += seg.secs;
      }
      m.set(d.date, secs);
    }
    return m;
  }, [days, superCatIds]);
  const appAll = useAppRangeDetail(app?.groupId ?? null, from, today, selectedDeviceId);
  const appSecs = useMemo(
    () => new Map((appAll.data?.buckets ?? []).map((b) => [b.key, b.secs])),
    [appAll.data],
  );
  const totalMinutes = Math.round(
    days.reduce((sum, d) => sum + d.segments.reduce((s, x) => s + x.secs, 0), 0) / 60,
  );

  const dateFmt = useMemo(
    () =>
      new Intl.DateTimeFormat(i18n.language, {
        year: "numeric",
        month: "short",
        day: "numeric",
      }),
    [i18n.language],
  );
  const dayFmt = useMemo(
    () =>
      new Intl.DateTimeFormat(i18n.language, {
        year: "numeric",
        month: "short",
        day: "numeric",
        weekday: "short",
      }),
    [i18n.language],
  );
  const monthFmt = useMemo(
    () => new Intl.DateTimeFormat(i18n.language, { year: "numeric", month: "long" }),
    [i18n.language],
  );

  if (earliest === null) return <EmptyHint message={t("history.empty")} />;
  if (from === null || domain === null || view === null || daysLoad.latest === null) {
    // 拉失败时 loading 已经结束、却没有数据
    return from === null || daysLoad.loading ? (
      <div className={styles.loading}>{t("history.loading")}</div>
    ) : (
      <EmptyHint />
    );
  }

  const nav: PeriodNav =
    tab === "year"
      ? {
          label: String(year),
          isCurrent: year === thisYear,
          currentTooltip: t("history.nav.backToThisYear"),
          prevLabel: t("history.nav.prevYear"),
          nextLabel: t("history.nav.nextYear"),
          canPrev: year > minYear,
          canNext: year < thisYear,
          onPrev: () => changeYear(year - 1),
          onNext: () => changeYear(year + 1),
          onCurrent: () => changeYear(thisYear),
        }
      : {
          label: monthFmt.format(new Date(mYear, mMonth, 1)),
          isCurrent: monthIdx === thisMonthIdx,
          currentTooltip: t("month.monthNav.backToThisMonth"),
          prevLabel: t("month.monthNav.prev"),
          nextLabel: t("month.monthNav.next"),
          canPrev: monthIdx > minMonthIdx,
          canNext: monthIdx < thisMonthIdx,
          onPrev: () => changeMonth(monthIdx - 1),
          onNext: () => changeMonth(monthIdx + 1),
          onCurrent: () => changeMonth(thisMonthIdx),
        };

  // 路径放在图表卡片标题后面，不单独占一行
  const path = (
    <nav className={styles.path} aria-label={t("history.path.label")}>
      <button
        type="button"
        className={styles.crumb}
        disabled={selectedSuper === null && activeApp === null}
        onClick={() => {
          setSuperId(null);
          closeApp();
        }}
      >
        {t("history.path.all")}
      </button>
      {selectedSuper && (
        <>
          <ChevronRight size={14} strokeWidth={2} className={styles.crumbSep} aria-hidden />
          <button
            type="button"
            className={styles.crumb}
            disabled={activeApp === null}
            onClick={closeApp}
          >
            {selectedSuper.name}
          </button>
        </>
      )}
      {activeApp && (
        <>
          <ChevronRight size={14} strokeWidth={2} className={styles.crumbSep} aria-hidden />
          <span className={styles.crumbCurrent}>{activeApp.name}</span>
        </>
      )}
      {zoom && (
        <span className={styles.chip}>
          {dateFmt.format(parseDayKey(zoom.from))} – {dateFmt.format(parseDayKey(zoom.to))}
          <button
            type="button"
            className={styles.chipClear}
            onClick={() => setZoom(null)}
            aria-label={t("history.chart.resetZoom")}
            title={t("history.chart.resetZoom")}
          >
            <X size={12} strokeWidth={2.2} />
          </button>
        </span>
      )}
      {day !== null && (
        <span className={styles.chip}>
          {dayFmt.format(parseDayKey(day))}
          <button
            type="button"
            className={styles.chipClear}
            onClick={() => setDay(null)}
            aria-label={t("history.path.clearDay")}
            title={t("history.path.clearDay")}
          >
            <X size={12} strokeWidth={2.2} />
          </button>
        </span>
      )}
    </nav>
  );

  const chartSecs = activeApp ? appSecs : overviewSecs;
  const chartColor = activeApp ? activeApp.color : (selectedSuper?.color ?? ACCENT);
  const chart = (
    <HistoryChartCard
      title={t("history.chart.title")}
      titleExtras={path}
      tab={tab}
      onTabChange={changeTab}
      nav={nav}
    >
      {tab === "month" ? (
        <MonthCalendar
          year={mYear}
          month={mMonth}
          secsByDay={chartSecs}
          from={from}
          to={today}
          color={chartColor}
          selectedKey={day}
          onSelect={toggleDay}
        />
      ) : (
        <Heatmap
          range={zoom ?? yearRange}
          secsByDay={chartSecs}
          from={from}
          to={today}
          color={chartColor}
          selectedKey={day}
          zoomed={zoom !== null}
          onSelect={toggleDay}
          onZoom={zoomTo}
          onResetZoom={resetZoom}
        />
      )}
    </HistoryChartCard>
  );

  return (
    <div className={styles.stats}>
      {/* 「共 X 小时 · 某天起」放到页面标题下面，跟日统计的小字同一个位置 */}
      {metaSlot &&
        createPortal(
          t("history.meta", {
            total: fmtHM(totalMinutes),
            since: dateFmt.format(parseDayKey(from)),
          }),
          metaSlot,
        )}
      {chart}

      <div className={styles.carousel}>
        <div
          className={`${styles.track} ${open ? styles.trackShifted : ""}`}
          onTransitionEnd={(e) => {
            if (e.target === e.currentTarget && !open) setApp(null);
          }}
        >
          <section className={styles.card} inert={open}>
            <header className={styles.cardHead}>
              <h2 className={styles.cardTitle}>{t("history.superCategories")}</h2>
              <span className={styles.cardTotal}>{fmtHM(scopedBreakdown.total)}</span>
            </header>
            {superItems.length > 0 ? (
              <ScrollBox fill>
                <RankedList
                  items={superItems}
                  selectedId={superId}
                  onItemClick={(item) =>
                    setSuperId((prev) => (prev === item.id ? null : item.id))
                  }
                />
              </ScrollBox>
            ) : (
              <EmptyHint />
            )}
          </section>

          <section className={`${styles.card} ${appsLoad.loading ? styles.stale : ""}`}>
            <header className={styles.cardHead}>
              <h2 className={styles.cardTitle}>{t("history.apps")}</h2>
              <span className={styles.cardTotal}>{fmtHM(appsMinutes)}</span>
            </header>
            {appRanks.length > 0 ? (
              <ScrollBox fill>
                <RankedList
                  items={appRanks}
                  selectedId={activeApp?.groupId ?? null}
                  onItemClick={clickApp}
                />
              </ScrollBox>
            ) : appsLoad.loading ? (
              <div className={styles.loading}>{t("history.loading")}</div>
            ) : (
              <EmptyHint />
            )}
          </section>

          {app ? (
            <div className={styles.detailSlot} inert={!open}>
              <AppDetailCard
                app={app}
                all={appAll.data}
                allRange={{ from, to: today }}
                scope={{ from: scopeFrom ?? from, to: scopeTo ?? today }}
                selectedDay={day}
                deviceId={selectedDeviceId}
                onClose={closeApp}
              />
            </div>
          ) : (
            <div />
          )}
        </div>
      </div>
    </div>
  );
}
