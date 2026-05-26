import { useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { useSettings } from "../../state/settings";
import { useCategories } from "../../state/categories";
import { DevicePicker } from "../../components/DevicePicker/DevicePicker";
import { ScrollBox } from "../../components/ScrollBox/ScrollBox";
import { PeriodCard } from "../../components/PeriodCard/PeriodCard";
import { PeriodLegend } from "../../components/PeriodLegend/PeriodLegend";
import { EmptyHint } from "../../components/EmptyHint/EmptyHint";
import { HourlyChart, type WorkRange } from "./HourlyChart";
import { RankedList } from "./RankedList";
import { ViewToggle, type StatsView } from "./ViewToggle";
import { PieView } from "./PieView";
import { PieDrillDetail } from "./PieDrillDetail";
import { useDayCache } from "../../hooks/useDayCache";
import { useHourApps } from "../../hooks/useHourApps";
import { useClickOutsideBars } from "../../hooks/useClickOutsideBars";
import { useDeviceFilter } from "../../state/deviceFilter";
import { usePeriodNavigation } from "../../hooks/usePeriodNavigation";
import { usePeriodRankings } from "../../hooks/usePeriodRankings";
import {
  useSuperCategoryBreakdown,
  catMinutesFromSegments,
} from "../../hooks/useSuperCategoryBreakdown";
import { useDurationFormatter } from "../../utils/duration";
import { withViewTransition } from "../../utils/viewTransition";
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
  const { categories } = useCategories();
  const fmtHM = useDurationFormatter();

  /** 「时段 / 占比」segmented；默认 "bars" 保留现有行为。 */
  const [view, setView] = useState<StatsView>("bars");
  /** 占比 drill：当前选中的 super-id；null 表示列表层。 */
  const [drillId, setDrillId] = useState<string | null>(null);
  // 切日 / 切设备 → 自动回列表层（防止 drill 状态跨日跨设备 stale）
  useEffect(() => {
    setDrillId(null);
  }, [offset, selectedDeviceId]);

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

  // —— 占比视图三 slide 的 super-category 聚合 —— //
  // prev/curr/next 三天分别算 cat-minutes，再各跑一遍 useSuperCategoryBreakdown
  // （三次 hook 调用顺序稳定，符合 hooks 规则）。prev/next 不参与 drill，所以
  // 只需要 slices/total，不需要 cats 详情。
  const prevHoursData = useMemo(() => getDay(offset - 1).hours, [getDay, offset]);
  const nextHoursData = useMemo(() => getDay(offset + 1).hours, [getDay, offset]);
  const prevCatMinutes = useMemo(() => catMinutesFromSegments(prevHoursData), [prevHoursData]);
  const currCatMinutes = useMemo(() => catMinutesFromSegments(hours), [hours]);
  const nextCatMinutes = useMemo(() => catMinutesFromSegments(nextHoursData), [nextHoursData]);
  const prevBreakdown = useSuperCategoryBreakdown(prevCatMinutes);
  const currBreakdown = useSuperCategoryBreakdown(currCatMinutes);
  const nextBreakdown = useSuperCategoryBreakdown(nextCatMinutes);

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
        title={view === "bars" ? t("today.chart.title") : t("today.pie.cardTitle")}
        headLeftExtras={
          <ViewToggle
            view={view}
            onChange={(v) => withViewTransition(() => setView(v))}
          />
        }
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
          view === "bars" ? (
            <PeriodLegend
              workHoursLabel={workRanges ? t("today.legend.workHours") : undefined}
            />
          ) : null
        }
        slides={
          view === "bars"
            ? [
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
              ]
            : [
                // 占比视图三 slide：prev/next 渲染但 interactive=false，不挂 view-transition-name
                <PieView
                  key={`pie-prev-${offset - 1}`}
                  slices={prevBreakdown.slices}
                  total={prevBreakdown.total}
                  interactive={false}
                />,
                drillId !== null &&
                currBreakdown.slices.find((s) => s.id === drillId) ? (
                  <PieDrillDetail
                    key={`pie-drill-${offset}-${drillId}`}
                    slice={currBreakdown.slices.find((s) => s.id === drillId)!}
                    grandTotal={currBreakdown.total}
                    apps={apps}
                    cats={categories}
                    onBack={() => setDrillId(null)}
                  />
                ) : (
                  <PieView
                    key={`pie-curr-${offset}`}
                    slices={currBreakdown.slices}
                    total={currBreakdown.total}
                    onDrill={setDrillId}
                  />
                ),
                <PieView
                  key={`pie-next-${offset + 1}`}
                  slices={nextBreakdown.slices}
                  total={nextBreakdown.total}
                  interactive={false}
                />,
              ]
        }
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
