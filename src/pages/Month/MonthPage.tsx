import { useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { DevicePicker } from "../../components/DevicePicker/DevicePicker";
import { ScrollBox } from "../../components/ScrollBox/ScrollBox";
import { PeriodCard } from "../../components/PeriodCard/PeriodCard";
import { PeriodLegend } from "../../components/PeriodLegend/PeriodLegend";
import { EmptyHint } from "../../components/EmptyHint/EmptyHint";
import { useMonthCache } from "../../hooks/useMonthCache";
import { useSelectedDayApps } from "../../hooks/useSelectedDayApps";
import { useClickOutsideBars } from "../../hooks/useClickOutsideBars";
import { useDeviceFilter } from "../../state/deviceFilter";
import { usePeriodNavigation } from "../../hooks/usePeriodNavigation";
import { usePeriodRankings } from "../../hooks/usePeriodRankings";
import { useDurationFormatter } from "../../utils/duration";
import { DailyBarChart } from "../Week/DailyBarChart";
import { RankedList } from "../Today/RankedList";
import type { DaySummary } from "../../api/hindsight";
import styles from "./MonthPage.module.css";

/** 见 WeekPage 同名函数：days[i].date → 相对今天的 dayOffset。 */
function dayOffsetForDate(date: Date): number {
  const a = new Date(date);
  a.setHours(0, 0, 0, 0);
  const b = new Date();
  b.setHours(0, 0, 0, 0);
  return Math.round((a.getTime() - b.getTime()) / 86400000);
}

export default function MonthPage() {
  const { t, i18n } = useTranslation();
  const { selectedDeviceId } = useDeviceFilter();
  const { offset, delta, transitioning, canGoForward, frameRef, commit, jumpToCurrent } =
    usePeriodNavigation();
  const { get: getMonth } = useMonthCache(offset, selectedDeviceId);
  const fmtHM = useDurationFormatter();

  const { days, apps } = useMemo(() => getMonth(offset), [getMonth, offset]);

  // 月份显示文案：中文取数字 1-12，英文取本地化月份名（如 "May"）
  const fmtMonth = (list: DaySummary[], off: number): string => {
    const base =
      list.length > 0
        ? list[0].date
        : new Date(new Date().getFullYear(), new Date().getMonth() + off, 1);
    const isZh = i18n.language.startsWith("zh");
    const monthText = isZh
      ? String(base.getMonth() + 1)
      : new Intl.DateTimeFormat(i18n.language, { month: "long" }).format(base);
    return t("month.monthLabel", {
      year: base.getFullYear(),
      month: monthText,
    });
  };

  // 月切换 pill 的本地化文案
  const monthPillLabel = (off: number): string => {
    if (off === 0) return t("month.monthNav.thisMonth");
    if (off === -1) return t("month.monthNav.lastMonth");
    if (off < -1) return t("month.monthNav.monthsAgo", { count: -off });
    return t("month.monthNav.monthsLater", { count: off });
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

  // 点某天 → 高亮 + 筛排行；toggle；offset / device 切换时清
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
      ? t("month.selection.label", {
          month: selectedDay.date.getMonth() + 1,
          day: selectedDay.date.getDate(),
        })
      : null;

  /** 月度 X 轴：每周一标一次（每 7 天）+ 月底最后一天 */
  const buildXLabel =
    (slideDays: DaySummary[]) =>
    (_d: DaySummary, i: number): string | null => {
      const day = slideDays[i].date.getDate();
      if (day === 1 || day % 7 === 0 || i === slideDays.length - 1) {
        return String(day);
      }
      return null;
    };

  const slideDaysList = [offset - 1, offset, offset + 1].map((o) =>
    o === offset ? days : getMonth(o).days,
  );

  return (
    <div className={styles.page}>
      <header className={styles.header}>
        <h1 className={styles.title}>{t("month.title")}</h1>
        <p className={styles.meta}>
          {t("month.meta", {
            month: fmtMonth(days, offset),
            total: fmtHM(totalMinutes),
            avg: fmtHM(Math.round(avgPerDay)),
          })}
        </p>
      </header>

      <PeriodCard
        title={t("month.chart.title")}
        pillLabel={monthPillLabel(offset)}
        pillTooltip={t("month.monthNav.backToThisMonth")}
        prevAriaLabel={t("month.monthNav.prev")}
        nextAriaLabel={t("month.monthNav.next")}
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
          <DailyBarChart
            key="prev"
            days={slideDaysList[0]}
            xLabel={buildXLabel(slideDaysList[0])}
          />,
          <DailyBarChart
            key="current"
            days={slideDaysList[1]}
            xLabel={buildXLabel(slideDaysList[1])}
            selectedIndex={selectedIndex}
            onIndexClick={handleDayClick}
          />,
          <DailyBarChart
            key="next"
            days={slideDaysList[2]}
            xLabel={buildXLabel(slideDaysList[2])}
          />,
        ]}
      />

      <div className={styles.ranks}>
        <section className={styles.card}>
          <header className={styles.cardHead}>
            <h2 className={styles.cardTitle}>{t("month.ranks.topApps")}</h2>
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
            <h2 className={styles.cardTitle}>{t("month.ranks.topCategories")}</h2>
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
