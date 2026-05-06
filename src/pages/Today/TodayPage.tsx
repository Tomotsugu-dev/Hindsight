import { useMemo } from "react";
import { useTranslation } from "react-i18next";
import { useSettings } from "../../state/settings";
import { DevicePicker } from "../../components/DevicePicker/DevicePicker";
import { ScrollBox } from "../../components/ScrollBox/ScrollBox";
import { PeriodCard } from "../../components/PeriodCard/PeriodCard";
import { PeriodLegend } from "../../components/PeriodLegend/PeriodLegend";
import { EmptyHint } from "../../components/EmptyHint/EmptyHint";
import { HourlyChart, type WorkRange } from "./HourlyChart";
import { RankedList } from "./RankedList";
import { useDayCache } from "../../hooks/useDayCache";
import { useDeviceFilter } from "../../state/deviceFilter";
import { usePeriodNavigation } from "../../hooks/usePeriodNavigation";
import { usePeriodRankings } from "../../hooks/usePeriodRankings";
import { useDurationFormatter } from "../../utils/duration";
import styles from "./TodayPage.module.css";

function parseHM(s: string): number {
  const [h, m] = s.split(":").map((p) => parseInt(p, 10));
  if (Number.isNaN(h)) return 0;
  return h + (Number.isNaN(m) ? 0 : m / 60);
}

function fmtDate(d: Date): string {
  return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}-${String(d.getDate()).padStart(2, "0")}`;
}

function dateForOffset(offset: number): Date {
  const d = new Date();
  d.setDate(d.getDate() + offset);
  return d;
}

export default function TodayPage() {
  const { t } = useTranslation();
  const { selectedDeviceId } = useDeviceFilter();
  const { offset, delta, transitioning, canGoForward, frameRef, commit, jumpToCurrent } =
    usePeriodNavigation();
  const { get: getDay } = useDayCache(offset, selectedDeviceId);
  const { settings } = useSettings();
  const fmtHM = useDurationFormatter();

  // 日期切换 pill 的本地化文案
  const dayLabel = (off: number): string => {
    if (off === 0) return t("today.dayNav.today");
    if (off === -1) return t("today.dayNav.yesterday");
    if (off < -1) return t("today.dayNav.daysAgo", { count: -off });
    return t("today.dayNav.daysLater", { count: off });
  };

  const { hours, apps } = useMemo(() => getDay(offset), [getDay, offset]);

  const workRanges: WorkRange[] | null = useMemo(() => {
    if (!settings?.workHoursEnabled) return null;
    if (!settings.workRanges.length) return null;
    return settings.workRanges.map((r) => ({
      startHour: parseHM(r.start),
      endHour: parseHM(r.end),
    }));
  }, [settings]);

  const totalMinutes = useMemo(
    () =>
      hours.reduce(
        (sum, h) => sum + h.segments.reduce((s, x) => s + x.minutes, 0),
        0,
      ),
    [hours],
  );

  const { categoryRanks, appRanks } = usePeriodRankings(hours, apps);

  return (
    <div className={styles.page}>
      <header className={styles.header}>
        <h1 className={styles.title}>{t("today.title")}</h1>
        <p className={styles.meta}>
          {t("today.meta", {
            date: fmtDate(dateForOffset(offset)),
            duration: fmtHM(totalMinutes),
          })}
        </p>
      </header>

      <PeriodCard
        title={t("today.chart.title")}
        pillLabel={dayLabel(offset)}
        pillTooltip={t("today.dayNav.backToToday")}
        prevAriaLabel={t("today.dayNav.prev")}
        nextAriaLabel={t("today.dayNav.next")}
        offset={offset}
        transitioning={transitioning}
        delta={delta}
        frameRef={frameRef}
        canGoForward={canGoForward}
        onPrev={() => commit(-1)}
        onNext={() => commit(1)}
        onJumpToCurrent={jumpToCurrent}
        rightExtras={<DevicePicker />}
        footer={
          <PeriodLegend
            workHoursLabel={workRanges ? t("today.legend.workHours") : undefined}
          />
        }
        slides={[
          <HourlyChart key="prev" hours={getDay(offset - 1).hours} workHours={workRanges} />,
          <HourlyChart key="current" hours={hours} workHours={workRanges} />,
          <HourlyChart key="next" hours={getDay(offset + 1).hours} workHours={workRanges} />,
        ]}
      />

      <div className={styles.ranks}>
        <section className={styles.card}>
          <header className={styles.cardHead}>
            <h2 className={styles.cardTitle}>{t("today.ranks.topApps")}</h2>
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
            <h2 className={styles.cardTitle}>{t("today.ranks.topCategories")}</h2>
          </header>
          {categoryRanks.length > 0 ? (
            <div className={styles.rankBody}>
              <RankedList items={categoryRanks} />
            </div>
          ) : (
            <EmptyHint />
          )}
        </section>
      </div>
    </div>
  );
}
