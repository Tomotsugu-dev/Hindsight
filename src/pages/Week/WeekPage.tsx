import { useMemo } from "react";
import { useTranslation } from "react-i18next";
import { DevicePicker } from "../../components/DevicePicker/DevicePicker";
import { ScrollBox } from "../../components/ScrollBox/ScrollBox";
import { PeriodCard } from "../../components/PeriodCard/PeriodCard";
import { PeriodLegend } from "../../components/PeriodLegend/PeriodLegend";
import { EmptyHint } from "../../components/EmptyHint/EmptyHint";
import { useWeekCache } from "../../hooks/useWeekCache";
import { useDeviceFilter } from "../../state/deviceFilter";
import { usePeriodNavigation } from "../../hooks/usePeriodNavigation";
import { usePeriodRankings } from "../../hooks/usePeriodRankings";
import { useDurationFormatter } from "../../utils/duration";
import { WeeklyBarChart } from "./WeeklyBarChart";
import { RankedList } from "../Today/RankedList";
import type { DaySummary } from "../../api/hindsight";
import styles from "./WeekPage.module.css";

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

  const { categoryRanks, appRanks } = usePeriodRankings(days, apps);

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
          <WeeklyBarChart key="current" days={days} />,
          <WeeklyBarChart key="next" days={getWeek(offset + 1).days} />,
        ]}
      />

      <div className={styles.ranks}>
        <section className={styles.card}>
          <header className={styles.cardHead}>
            <h2 className={styles.cardTitle}>{t("week.ranks.topApps")}</h2>
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
