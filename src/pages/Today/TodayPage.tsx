import { useCallback, useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { useSettings } from "../../state/settings";
import { DevicePicker } from "../../components/DevicePicker/DevicePicker";
import { ScrollBox } from "../../components/ScrollBox/ScrollBox";
import { PeriodCard } from "../../components/PeriodCard/PeriodCard";
import { PeriodLegend } from "../../components/PeriodLegend/PeriodLegend";
import { EmptyHint } from "../../components/EmptyHint/EmptyHint";
import { InsightTiles } from "../../components/InsightTiles/InsightTiles";
import { HourlyChart, type WorkRange } from "./HourlyChart";
import { RankedList } from "../../components/RankedList/RankedList";
import { ViewToggle, type StatsView } from "../../components/ViewToggle/ViewToggle";
import { PieView } from "../../components/PieView/PieView";
import { useDayCache } from "../../hooks/useDayCache";
import { useHourApps } from "../../hooks/useHourApps";
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
import type { HourSlot } from "../../api/hindsight";
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
  const { t, i18n } = useTranslation();
  const { selectedDeviceId } = useDeviceFilter();
  const { offset, delta, transitioning, canGoForward, frameRef, commit, jumpToCurrent } =
    usePeriodNavigation();
  const { get: getDay } = useDayCache(offset, selectedDeviceId);
  const { settings } = useSettings();
  const fmtHM = useDurationFormatter();

  /** 「时段 / 占比」segmented；默认 "bars" 保留现有行为。 */
  const [view, setView] = useState<StatsView>("bars");
  /** 占比 drill：当前选中的 super-id；null 表示列表层。 */
  const [drillId, setDrillId] = useState<string | null>(null);
  // 切日 / 切设备 / 切视图 → 自动回列表层。view 进 deps 是为了：用户在占比里
  // pin 了某大类后切回时段视图，drill 状态在 UI 上已不可见但仍 pinned，再切回
  // 占比会"幽灵高亮"在上次的大类上。切视图时清掉，回占比是干净的列表层。
  useEffect(() => {
    setDrillId(null);
  }, [offset, selectedDeviceId, view]);

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
  // 跟 segmentsForRanks 同 scope 的总时长：选中小时时就是该小时总和，否则等于
  // totalMinutes（全日）。卡片右上角"总时长"显示用这个值才跟下方 apps 列表对齐。
  const scopedMinutes = useMemo(
    () =>
      segmentsForRanks.reduce(
        (sum, h) => sum + h.segments.reduce((s, x) => s + x.minutes, 0),
        0,
      ),
    [segmentsForRanks],
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

  // drill 状态下：底部两卡片同步缩进到该大类范围
  // - 标题改为「主要应用」/「分类构成」（复用 PieDrillDetail 已有 i18n key）
  // - app 排行只留 categoryId 命中该大类 cats 的；category 排行只留该大类下属
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
    : t("today.ranks.topApps");
  const categoriesTitle = drilledSlice
    ? t("today.pie.drill.categoriesTitle")
    : t("today.ranks.topCategories");

  // 顶部洞察行：当期 vs 上期 · 峰值小时 · 主力大类
  // drill 时该大类视角；上期同 super-cat lookup
  const peakLabelForHour = useCallback(
    (slot: HourSlot) => `${String(slot.hour).padStart(2, "0")}:00`,
    [],
  );
  const prevDrilledSlice = useMemo(
    () =>
      drilledSlice
        ? prevBreakdown.slices.find((s) => s.id === drilledSlice.id) ?? null
        : null,
    [drilledSlice, prevBreakdown],
  );
  const insights = usePeriodInsights({
    curr: hours,
    prev: prevHoursData,
    buildPeakLabel: peakLabelForHour,
    topSlice: currBreakdown.slices[0] ?? null,
    currTotal: totalMinutes,
    drill: drilledSlice
      ? { slice: drilledSlice, prevSlice: prevDrilledSlice }
      : undefined,
  });

  return (
    <div className={styles.page}>
      <header className={styles.header}>
        <h1 className={styles.title}>{t("today.title")}</h1>
        <p className={styles.meta}>
          {t("today.meta", {
            date: fmtDate(dateForOffset(offset)),
            weekday: new Intl.DateTimeFormat(i18n.language, {
              weekday: "short",
            }).format(dateForOffset(offset)),
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
                // 当前 slide 始终是 PieView；点击 toggle drillId，pin 住高亮，下方两卡按它过滤
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
          scope="today"
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
              {/* 总活动时间：
                  - 选中某小时 → 该小时总时长（跟下方 apps 列表 scope 一致）
                  - 否则 drill 时 → 该大类小计
                  - 否则 → 全日总时长
                  选中优先于 drill，因为用户对"选了再看时间"的直觉是"那个小时多少分钟" */}
              <span className={styles.cardTotal}>
                {fmtHM(
                  selectedHour !== null
                    ? scopedMinutes
                    : drilledSlice
                      ? drilledSlice.minutes
                      : totalMinutes,
                )}
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
            <div className={styles.rankBody}>
              <RankedList items={displayedCategoryRanks} />
            </div>
          ) : (
            <EmptyHint />
          )}
        </section>
      </div>
    </div>
  );
}
