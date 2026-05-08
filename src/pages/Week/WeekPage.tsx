import { useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { DevicePicker } from "../../components/DevicePicker/DevicePicker";
import { ScrollBox } from "../../components/ScrollBox/ScrollBox";
import { PeriodCard } from "../../components/PeriodCard/PeriodCard";
import { PeriodLegend } from "../../components/PeriodLegend/PeriodLegend";
import { EmptyHint } from "../../components/EmptyHint/EmptyHint";
import { useWeekCache } from "../../hooks/useWeekCache";
import { useSelectedDayApps } from "../../hooks/useSelectedDayApps";
import { useClickOutsideBars } from "../../hooks/useClickOutsideBars";
import { useDeviceFilter } from "../../state/deviceFilter";
import { usePeriodNavigation } from "../../hooks/usePeriodNavigation";
import { usePeriodRankings } from "../../hooks/usePeriodRankings";
import { useDurationFormatter } from "../../utils/duration";
import { WeeklyBarChart } from "./WeeklyBarChart";
import { RankedList } from "../Today/RankedList";
import type { DaySummary } from "../../api/hindsight";
import styles from "./WeekPage.module.css";

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

  const selectionLabel =
    selectedDay !== null
      ? t("week.selection.label", {
          month: selectedDay.date.getMonth() + 1,
          day: selectedDay.date.getDate(),
        })
      : null;

  return (
    <div className={styles.page}>
      <header className={styles.header}>
        <h1 className={styles.title}>{t("week.title")}</h1>
        <p className={styles.meta}>
          {t("week.meta", {
            range: fmtRange(days),
            total: fmtHM(totalMinutes),
            avg: fmtHM(Math.round(avgPerDay)),
          })}
        </p>
      </header>

      <PeriodCard
        title={t("week.chart.title")}
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
        footer={<PeriodLegend />}
        slides={[
          <WeeklyBarChart key="prev" days={getWeek(offset - 1).days} />,
          <WeeklyBarChart
            key="current"
            days={days}
            selectedIndex={selectedIndex}
            onIndexClick={handleDayClick}
          />,
          <WeeklyBarChart key="next" days={getWeek(offset + 1).days} />,
        ]}
      />

      <div className={styles.ranks}>
        <section className={styles.card}>
          <header className={styles.cardHead}>
            <h2 className={styles.cardTitle}>{t("week.ranks.topApps")}</h2>
            {selectionLabel && (
              <span className={styles.selectionLabel}>{selectionLabel}</span>
            )}
          </header>
          {appRanks.length > 0 ? (
            <ScrollBox maxHeight={280}>
              <RankedList items={appRanks} />
            </ScrollBox>
          ) : (
            <EmptyHint />
          )}
        </section>

        <section className={styles.card}>
          <header className={styles.cardHead}>
            <h2 className={styles.cardTitle}>{t("week.ranks.topCategories")}</h2>
            {selectionLabel && (
              <span className={styles.selectionLabel}>{selectionLabel}</span>
            )}
          </header>
          {categoryRanks.length > 0 ? (
            <ScrollBox maxHeight={280}>
              <RankedList items={categoryRanks} />
            </ScrollBox>
          ) : (
            <EmptyHint />
          )}
        </section>
      </div>
    </div>
  );
}
