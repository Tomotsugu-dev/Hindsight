import { useEffect, useMemo, useState } from "react";
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
import { useHourApps } from "../../hooks/useHourApps";
import { useClickOutsideBars } from "../../hooks/useClickOutsideBars";
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

  // 点柱子→选中那个小时；再点同一柱子取消（toggle）。
  // offset / device 切换时自动清，避免上一段选择跨日生效。
  const [selectedHour, setSelectedHour] = useState<number | null>(null);
  useEffect(() => {
    setSelectedHour(null);
  }, [offset, selectedDeviceId]);
  const handleHourClick = (h: number) =>
    setSelectedHour((prev) => (prev === h ? null : h));
  // 点页面任何非柱子区域 → 清除选中
  useClickOutsideBars(selectedHour !== null, () => setSelectedHour(null));

  // 选中小时时拉该小时的 top apps；未选中 → null/不请求
  const hourApps = useHourApps(offset, selectedHour, selectedDeviceId);

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

  // 选中小时时：categories segments 只用该小时；apps 用 hourApps（loading 时退到全日 apps，避免列表瞬空）
  const segmentsForRanks = useMemo(
    () => (selectedHour === null ? hours : hours.filter((h) => h.hour === selectedHour)),
    [hours, selectedHour],
  );
  const appsForRanks = useMemo(
    () => (selectedHour === null ? apps : (hourApps.apps ?? apps)),
    [selectedHour, apps, hourApps.apps],
  );
  const { categoryRanks, appRanks } = usePeriodRankings(
    segmentsForRanks,
    appsForRanks,
  );

  const selectionLabel =
    selectedHour !== null
      ? t("today.selection.label", {
          hour: String(selectedHour).padStart(2, "0"),
        })
      : null;

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
          // prev/next 是 PeriodCard 的滑动副本，不参与点击；只 current 接 onHourClick
          <HourlyChart key="prev" hours={getDay(offset - 1).hours} workHours={workRanges} />,
          <HourlyChart
            key="current"
            hours={hours}
            workHours={workRanges}
            selectedHour={selectedHour}
            onHourClick={handleHourClick}
          />,
          <HourlyChart key="next" hours={getDay(offset + 1).hours} workHours={workRanges} />,
        ]}
      />

      <div className={styles.ranks}>
        <section className={styles.card}>
          <header className={styles.cardHead}>
            <h2 className={styles.cardTitle}>{t("today.ranks.topApps")}</h2>
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
            <h2 className={styles.cardTitle}>{t("today.ranks.topCategories")}</h2>
            {selectionLabel && (
              <span className={styles.selectionLabel}>{selectionLabel}</span>
            )}
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
