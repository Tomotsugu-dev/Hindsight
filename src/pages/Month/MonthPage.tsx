import { useCallback, useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { DevicePicker } from "../../components/DevicePicker/DevicePicker";
import { ScrollBox } from "../../components/ScrollBox/ScrollBox";
import { PeriodCard } from "../../components/PeriodCard/PeriodCard";
import { PeriodLegend } from "../../components/PeriodLegend/PeriodLegend";
import { EmptyHint } from "../../components/EmptyHint/EmptyHint";
import { InsightTiles } from "../../components/InsightTiles/InsightTiles";
import { useMonthCache } from "../../hooks/useMonthCache";
import { useSelectedDayApps } from "../../hooks/useSelectedDayApps";
import { useClickOutsideBars } from "../../hooks/useClickOutsideBars";
import { useDeviceFilter } from "../../state/deviceFilter";
import { usePeriodNavigation } from "../../hooks/usePeriodNavigation";
import { usePeriodRankings } from "../../hooks/usePeriodRankings";
import { usePeriodInsights } from "../../hooks/usePeriodInsights";
import {
  useSuperCategoryBreakdown,
  catMinutesFromSegments,
  completedDaysOf,
} from "../../hooks/useSuperCategoryBreakdown";
import { useDurationFormatter } from "../../utils/duration";
import { withViewTransition } from "../../utils/viewTransition";
import { DailyBarChart } from "../../components/DailyBarChart/DailyBarChart";
import { RankedList } from "../../components/RankedList/RankedList";
import {
  AppDetailDrawer,
  type AppDetailTarget,
} from "../../components/AppDetailDrawer/AppDetailDrawer";
import { ViewToggle, type StatsView } from "../../components/ViewToggle/ViewToggle";
import { PieView } from "../../components/PieView/PieView";
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

  /** 「时段 / 占比」segmented + drill state（跟 TodayPage 同款，见那边注释；
   *  切月不清 drill——钉着大类翻上/下月对比才是动线） */
  const [view, setView] = useState<StatsView>("bars");
  const [drillId, setDrillId] = useState<string | null>(null);
  useEffect(() => {
    setDrillId(null);
  }, [selectedDeviceId, view]);

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
      Math.round(
          days.reduce(
            (sum, d) => sum + d.segments.reduce((s, x) => s + x.secs, 0),
            0,
          ) / 60,
        ),
    [days],
  );

  // 「日均」统一口径(与周页一致):已完成自然天的总时长 ÷ 已完成天数——
  // 今天从分子分母一起排除;过去月份天然 = 当月全部天数。旧版除以"活跃天数"
  // 会把只用了 10 天电脑的月份日均虚高三倍,且与周页口径互相矛盾。
  // 每月 1 号(无完整天)→ undefined:顶部 meta 显示 —,日均 tile 整块隐藏。
  const completedDays = useMemo(() => completedDaysOf(days), [days]);
  const completedMinutes = useMemo(
    () =>
      Math.round(
        completedDays.reduce(
          (sum, d) => sum + d.segments.reduce((s, x) => s + x.secs, 0),
          0,
        ) / 60,
      ),
    [completedDays],
  );
  const avgPerCompletedDay =
    completedDays.length > 0
      ? Math.round(completedMinutes / completedDays.length)
      : undefined;

  // 点某天 → 高亮 + 筛排行；toggle；offset / device 切换时清
  const [selectedIndex, setSelectedIndex] = useState<number | null>(null);
  // 点应用 → 详情抽屉；切月 / 切设备时关掉
  const [selectedApp, setSelectedApp] = useState<AppDetailTarget | null>(null);
  useEffect(() => {
    setSelectedIndex(null);
    setSelectedApp(null);
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
  // 跟 segmentsForRanks 同 scope 的总时长：选中某天时就是该天总和，否则全月。
  // 卡片右上角"总时长"用这个值才跟下方 apps 列表对齐。
  const scopedMinutes = useMemo(
    () =>
      Math.round(
          segmentsForRanks.reduce(
            (sum, d) => sum + d.segments.reduce((s, x) => s + x.secs, 0),
            0,
          ) / 60,
        ),
    [segmentsForRanks],
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
          // 月度 30+ 天的密集柱图里光看 6/6 不直观，加星期帮用户判断"这是星期几"。
          // Intl.DateTimeFormat 跟系统 locale 走：zh→"周六" / ja→"土" / en→"Sat"
          weekday: new Intl.DateTimeFormat(i18n.language, {
            weekday: "short",
          }).format(selectedDay.date),
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

  // —— 占比视图三 slide 的 super-category 聚合 ——
  const prevCatMinutes = useMemo(() => catMinutesFromSegments(slideDaysList[0]), [slideDaysList]);
  const currCatMinutes = useMemo(() => catMinutesFromSegments(days), [days]);
  const nextCatMinutes = useMemo(() => catMinutesFromSegments(slideDaysList[2]), [slideDaysList]);
  const prevBreakdown = useSuperCategoryBreakdown(prevCatMinutes);
  const currBreakdown = useSuperCategoryBreakdown(currCatMinutes);
  const nextBreakdown = useSuperCategoryBreakdown(nextCatMinutes);
  // 日均 tile 的 drill 分子:只累计已完成天(饼图/总时长仍是全月口径含今天)
  const completedCatMinutes = useMemo(
    () => catMinutesFromSegments(completedDays),
    [completedDays],
  );
  const completedBreakdown = useSuperCategoryBreakdown(completedCatMinutes);
  // 上月是完整周期,分母 = 上月自然天数。整月无数据 → undefined(tile 显示 —)
  // 而不是 0,避免"↑ 满值 比上月"假涨幅
  const prevAvgPerTotalDays = useMemo(
    () =>
      prevBreakdown.total > 0 && slideDaysList[0].length > 0
        ? Math.round(prevBreakdown.total / slideDaysList[0].length)
        : undefined,
    [prevBreakdown.total, slideDaysList],
  );

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
    : t("month.ranks.topApps");
  const categoriesTitle = drilledSlice
    ? t("today.pie.drill.categoriesTitle")
    : t("month.ranks.topCategories");

  // 顶部洞察行：当期 vs 上月 · 峰值日 · 主力大类
  // drill 时该大类视角；上期同 super-cat lookup
  const peakLabelForDay = useCallback(
    (day: DaySummary) =>
      t("month.insights.peakDate", {
        month: day.date.getMonth() + 1,
        day: day.date.getDate(),
      }),
    [t],
  );
  const prevDrilledSlice = useMemo(
    () =>
      drilledSlice
        ? prevBreakdown.slices.find((s) => s.id === drilledSlice.id) ?? null
        : null,
    [drilledSlice, prevBreakdown],
  );
  // 日均 / 日均对比 tile 跟其它 tile 一样支持 drill：drill 时显示该大类的日均，
  // 分子分母都按已完成天口径(上月是完整周期,分母 = 上月自然天数)。
  // 上月没有该大类（prevDrilledSlice=null）按 0 算，对比语义即"从无到有"。
  const displayedAvgMinutes = drilledSlice
    ? completedDays.length > 0
      ? Math.round(
          (completedBreakdown.slices.find((s) => s.id === drilledSlice.id)
            ?.minutes ?? 0) / completedDays.length,
        )
      : undefined
    : avgPerCompletedDay;
  const displayedPrevAvgMinutes = drilledSlice
    ? prevBreakdown.total > 0 && slideDaysList[0].length > 0
      ? Math.round((prevDrilledSlice?.minutes ?? 0) / slideDaysList[0].length)
      : undefined // 上月整月无数据：显示 — 而非"从无到有"
    : prevAvgPerTotalDays;
  const insights = usePeriodInsights({
    curr: days,
    prev: slideDaysList[0],
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
        <h1 className={styles.title}>{t("month.title")}</h1>
        <p className={styles.meta}>
          {t("month.meta", {
            month: fmtMonth(days, offset),
            avg: avgPerCompletedDay != null ? fmtHM(avgPerCompletedDay) : "—",
          })}
        </p>
      </header>

      <PeriodCard
        title={view === "bars" ? t("month.chart.title") : t("today.pie.monthCardTitle")}
        headLeftExtras={
          <ViewToggle
            view={view}
            onChange={(v) => withViewTransition(() => setView(v))}
          />
        }
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
        footer={view === "bars" ? <PeriodLegend /> : null}
        slides={
          view === "bars"
            ? [
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
                  // drill 跨周期保留后 drillId 可能在当前周期没有对应切片（ghost）——
                  // 直接传会让 PieView 把所有行/圆环置灰且无高亮。解析得到才 pin。
                  pinnedId={drilledSlice ? drillId : null}
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
          scope="month"
          drilledSlice={drilledSlice}
          avgMinutes={displayedAvgMinutes}
          prevAvgMinutes={displayedPrevAvgMinutes}
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
              {/* 总活动时间（跟 TodayPage 同款语义）：
                  选中某天 → 该天总时长；否则 drill → 大类小计；否则全月。 */}
              <span className={styles.cardTotal}>
                {fmtHM(
                  selectedIndex !== null
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
              <RankedList
                items={displayedAppRanks}
                onItemClick={(item) =>
                  setSelectedApp({
                    name: item.name,
                    iconProcess: item.iconProcess ?? item.id,
                    categoryLabel: item.subtitle,
                    color: item.color,
                    minutes: item.minutes,
                  })
                }
              />
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

      <AppDetailDrawer
        app={selectedApp}
        scope="month"
        offset={offset}
        deviceId={selectedDeviceId}
        onClose={() => setSelectedApp(null)}
      />
    </div>
  );
}
