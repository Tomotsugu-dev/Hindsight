import { useCallback, useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { DevicePicker } from "../../components/DevicePicker/DevicePicker";
import { ScrollBox } from "../../components/ScrollBox/ScrollBox";
import { PeriodCard } from "../../components/PeriodCard/PeriodCard";
import { PeriodLegend } from "../../components/PeriodLegend/PeriodLegend";
import { EmptyHint } from "../../components/EmptyHint/EmptyHint";
import { InsightTiles } from "../../components/InsightTiles/InsightTiles";
import { useWeekCache } from "../../hooks/useWeekCache";
import { useSelectedDayApps } from "../../hooks/useSelectedDayApps";
import { useClickOutsideBars } from "../../hooks/useClickOutsideBars";
import { useDeviceFilter } from "../../state/deviceFilter";
import { usePeriodNavigation } from "../../hooks/usePeriodNavigation";
import { usePeriodRankings } from "../../hooks/usePeriodRankings";
import { usePeriodInsights } from "../../hooks/usePeriodInsights";
import {
  useSuperCategoryBreakdown,
  catMinutesFromSegments,
} from "../../hooks/useSuperCategoryBreakdown";
import { useDurationFormatter } from "../../utils/duration";
import { withViewTransition } from "../../utils/viewTransition";
import { WeeklyBarChart } from "./WeeklyBarChart";
import { RankedList } from "../Today/RankedList";
import { ViewToggle, type StatsView } from "../Today/ViewToggle";
import { PieView } from "../Today/PieView";
import type { DaySummary } from "../../api/hindsight";
import styles from "./WeekPage.module.css";

/** Sun..Sat → mon..sun key（i18n 里是星期一开头）。 */
const DOW_KEYS = ["sun", "mon", "tue", "wed", "thu", "fri", "sat"] as const;

/** 把 days[i].date 折算成相对今天的 dayOffset（0=今天，-1=昨天）。
 *  用 startOfDay 做差，避免时区/夏令时错位一格。 */
function dayOffsetForDate(date: Date): number {
  const a = new Date(date);
  a.setHours(0, 0, 0, 0);
  const b = new Date();
  b.setHours(0, 0, 0, 0);
  return Math.round((a.getTime() - b.getTime()) / 86400000);
}

export default function WeekPage() {
  const { t } = useTranslation();
  const { selectedDeviceId } = useDeviceFilter();
  const { offset, delta, transitioning, canGoForward, frameRef, commit, jumpToCurrent } =
    usePeriodNavigation();
  const { get: getWeek } = useWeekCache(offset, selectedDeviceId);
  const fmtHM = useDurationFormatter();

  const { days, apps } = useMemo(() => getWeek(offset), [getWeek, offset]);

  /** 「时段 / 占比」segmented + drill state（跟 TodayPage 同款，见那边注释） */
  const [view, setView] = useState<StatsView>("bars");
  const [drillId, setDrillId] = useState<string | null>(null);
  useEffect(() => {
    setDrillId(null);
  }, [offset, selectedDeviceId]);

  // 周日期范围文案
  const fmtRange = (list: DaySummary[]): string => {
    if (list.length === 0) return "";
    const first = list[0].date;
    const last = list[list.length - 1].date;
    const sameMonth = first.getMonth() === last.getMonth();
    if (sameMonth) {
      return t("week.rangeSameMonth", {
        month: first.getMonth() + 1,
        startDay: first.getDate(),
        endDay: last.getDate(),
      });
    }
    return t("week.rangeCrossMonth", {
      startMonth: first.getMonth() + 1,
      startDay: first.getDate(),
      endMonth: last.getMonth() + 1,
      endDay: last.getDate(),
    });
  };

  // 周切换 pill 的本地化文案
  const weekLabel = (off: number): string => {
    if (off === 0) return t("week.weekNav.thisWeek");
    if (off === -1) return t("week.weekNav.lastWeek");
    if (off < -1) return t("week.weekNav.weeksAgo", { count: -off });
    return t("week.weekNav.weeksLater", { count: off });
  };

  const totalMinutes = useMemo(
    () =>
      days.reduce(
        (sum, d) => sum + d.segments.reduce((s, x) => s + x.minutes, 0),
        0,
      ),
    [days],
  );

  const activeDays = useMemo(
    () => days.filter((d) => d.segments.length > 0).length,
    [days],
  );
  const avgPerDay = activeDays > 0 ? totalMinutes / activeDays : 0;

  // 点某天 → 该 day index 高亮，其它淡化；toggle
  const [selectedIndex, setSelectedIndex] = useState<number | null>(null);
  useEffect(() => {
    setSelectedIndex(null);
  }, [offset, selectedDeviceId]);
  const handleDayClick = (i: number) =>
    setSelectedIndex((prev) => (prev === i ? null : i));
  useClickOutsideBars(selectedIndex !== null, () => setSelectedIndex(null));

  const selectedDay = selectedIndex !== null ? days[selectedIndex] : null;
  const selectedDayOffset = selectedDay ? dayOffsetForDate(selectedDay.date) : null;
  const dayApps = useSelectedDayApps(selectedDayOffset, selectedDeviceId);

  const segmentsForRanks = useMemo(
    () =>
      selectedIndex === null || !days[selectedIndex] ? days : [days[selectedIndex]],
    [days, selectedIndex],
  );
  const appsForRanks = useMemo(
    () => (selectedIndex === null ? apps : (dayApps.apps ?? apps)),
    [selectedIndex, apps, dayApps.apps],
  );
  const { categoryRanks, appRanks } = usePeriodRankings(segmentsForRanks, appsForRanks);

  // —— 占比视图三 slide 的 super-category 聚合 ——
  const prevDaysData = useMemo(() => getWeek(offset - 1).days, [getWeek, offset]);
  const nextDaysData = useMemo(() => getWeek(offset + 1).days, [getWeek, offset]);
  const prevCatMinutes = useMemo(() => catMinutesFromSegments(prevDaysData), [prevDaysData]);
  const currCatMinutes = useMemo(() => catMinutesFromSegments(days), [days]);
  const nextCatMinutes = useMemo(() => catMinutesFromSegments(nextDaysData), [nextDaysData]);
  const prevBreakdown = useSuperCategoryBreakdown(prevCatMinutes);
  const currBreakdown = useSuperCategoryBreakdown(currCatMinutes);
  const nextBreakdown = useSuperCategoryBreakdown(nextCatMinutes);

  const selectionLabel =
    selectedDay !== null
      ? t("week.selection.label", {
          month: selectedDay.date.getMonth() + 1,
          day: selectedDay.date.getDate(),
        })
      : null;

  // drill 状态下：底部两卡片同步缩进到该大类范围（详见 TodayPage 同名块注释）
  const drilledSlice =
    drillId !== null
      ? currBreakdown.slices.find((s) => s.id === drillId) ?? null
      : null;
  const childCatIds = useMemo(
    () => (drilledSlice ? new Set(drilledSlice.cats.map((c) => c.id)) : null),
    [drilledSlice],
  );
  const displayedAppRanks = useMemo(
    () =>
      childCatIds
        ? appRanks.filter((r) => r.categoryId && childCatIds.has(r.categoryId))
        : appRanks,
    [appRanks, childCatIds],
  );
  const displayedCategoryRanks = useMemo(
    () =>
      childCatIds
        ? categoryRanks.filter((r) => childCatIds.has(r.id))
        : categoryRanks,
    [categoryRanks, childCatIds],
  );
  const appsTitle = drilledSlice
    ? t("today.pie.drill.appsTitle")
    : t("week.ranks.topApps");
  const categoriesTitle = drilledSlice
    ? t("today.pie.drill.categoriesTitle")
    : t("week.ranks.topCategories");

  // 顶部洞察行：当期 vs 上周 · 峰值日 · 主力大类
  // drill 时该大类视角；上期同 super-cat lookup
  const peakLabelForDay = useCallback(
    (day: DaySummary) => t(`week.dow.${DOW_KEYS[day.date.getDay()]}`),
    [t],
  );
  const prevDrilledSlice = useMemo(
    () =>
      drilledSlice
        ? prevBreakdown.slices.find((s) => s.id === drilledSlice.id) ?? null
        : null,
    [drilledSlice, prevBreakdown],
  );
  const insights = usePeriodInsights({
    curr: days,
    prev: prevDaysData,
    buildPeakLabel: peakLabelForDay,
    topSlice: currBreakdown.slices[0] ?? null,
    currTotal: totalMinutes,
    drill: drilledSlice
      ? { slice: drilledSlice, prevSlice: prevDrilledSlice }
      : undefined,
  });

  return (
    <div className={styles.page}>
      <header className={styles.header}>
        <h1 className={styles.title}>{t("week.title")}</h1>
        <p className={styles.meta}>
          {t("week.meta", {
            range: fmtRange(days),
            avg: fmtHM(Math.round(avgPerDay)),
          })}
        </p>
      </header>

      <PeriodCard
        title={view === "bars" ? t("week.chart.title") : t("today.pie.weekCardTitle")}
        headLeftExtras={
          <ViewToggle
            view={view}
            onChange={(v) => withViewTransition(() => setView(v))}
          />
        }
        pillLabel={weekLabel(offset)}
        pillTooltip={t("week.weekNav.backToThisWeek")}
        prevAriaLabel={t("week.weekNav.prev")}
        nextAriaLabel={t("week.weekNav.next")}
        offset={offset}
        transitioning={transitioning}
        delta={delta}
        frameRef={frameRef}
        canGoForward={canGoForward}
        onPrev={() => commit(-1)}
        onNext={() => commit(1)}
        onJumpToCurrent={jumpToCurrent}
        rightExtras={<DevicePicker />}
        footer={view === "bars" ? <PeriodLegend /> : null}
        slides={
          view === "bars"
            ? [
                <WeeklyBarChart key="prev" days={getWeek(offset - 1).days} />,
                <WeeklyBarChart
                  key="current"
                  days={days}
                  selectedIndex={selectedIndex}
                  onIndexClick={handleDayClick}
                />,
                <WeeklyBarChart key="next" days={getWeek(offset + 1).days} />,
              ]
            : [
                <PieView
                  key={`pie-prev-${offset - 1}`}
                  slices={prevBreakdown.slices}
                  total={prevBreakdown.total}
                  interactive={false}
                />,
                // 当前 slide 始终是 PieView；点击 toggle drillId（详见 TodayPage 同名块）
                <PieView
                  key={`pie-curr-${offset}`}
                  slices={currBreakdown.slices}
                  total={currBreakdown.total}
                  pinnedId={drillId}
                  onDrill={(id) =>
                    setDrillId((prev) => (prev === id ? null : id))
                  }
                />,
                <PieView
                  key={`pie-next-${offset + 1}`}
                  slices={nextBreakdown.slices}
                  total={nextBreakdown.total}
                  interactive={false}
                />,
              ]
        }
      />

      {/* 仅占比视图显示：tile 是饼图的数字摘要（drill 联动 / 主力 / 构成） */}
      {view === "pie" && (
        <InsightTiles
          insights={insights}
          scope="week"
          drilledSlice={drilledSlice}
        />
      )}

      <div className={styles.ranks}>
        <section className={styles.card}>
          <header className={styles.cardHead}>
            <h2 className={styles.cardTitle}>{appsTitle}</h2>
            <div className={styles.cardHeadRight}>
              {selectionLabel && (
                <span className={styles.selectionLabel}>{selectionLabel}</span>
              )}
              {/* 总活动时间：drill 时显示该大类小计，否则全周总时长 */}
              <span className={styles.cardTotal}>
                {fmtHM(drilledSlice ? drilledSlice.minutes : totalMinutes)}
              </span>
            </div>
          </header>
          {displayedAppRanks.length > 0 ? (
            <ScrollBox maxHeight={280}>
              <RankedList items={displayedAppRanks} />
            </ScrollBox>
          ) : (
            <EmptyHint />
          )}
        </section>

        <section className={styles.card}>
          <header className={styles.cardHead}>
            <h2 className={styles.cardTitle}>{categoriesTitle}</h2>
            {selectionLabel && (
              <span className={styles.selectionLabel}>{selectionLabel}</span>
            )}
          </header>
          {displayedCategoryRanks.length > 0 ? (
            <ScrollBox maxHeight={280}>
              <RankedList items={displayedCategoryRanks} />
            </ScrollBox>
          ) : (
            <EmptyHint />
          )}
        </section>
      </div>
    </div>
  );
}
