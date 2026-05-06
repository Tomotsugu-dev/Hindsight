import { useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { ChevronLeft, ChevronRight } from "lucide-react";
import { useCategories } from "../../state/categories";
import { AppIcon } from "../../components/AppIcon/AppIcon";
import { AppStack } from "../../components/AppStack/AppStack";
import { DevicePicker } from "../../components/DevicePicker/DevicePicker";
import { ScrollBox } from "../../components/ScrollBox/ScrollBox";
import { displayAppName } from "../../utils/displayName";
import { displayCategoryName } from "../../utils/categoryName";
import { useMonthCache } from "../../hooks/useMonthCache";
import { useDeviceFilter } from "../../state/deviceFilter";
import { useMouseGlow } from "../../hooks/useMouseGlow";
import { DailyBarChart } from "../Week/DailyBarChart";
import { RankedList, type RankedItem } from "../Today/RankedList";
import type { DaySummary } from "../../api/hindsight";
import styles from "./MonthPage.module.css";

const SWIPE_DURATION = 420;

export default function MonthPage() {
  const { t, i18n } = useTranslation();
  const [offset, setOffset] = useState(0);
  const { categories, getCategory } = useCategories();
  const { selectedDeviceId } = useDeviceFilter();
  const { get: getMonth } = useMonthCache(offset, selectedDeviceId);

  const { days, apps } = useMemo(() => getMonth(offset), [getMonth, offset]);

  // 时长格式化 —— 复用 common.duration.* 资源
  const fmtHM = (min: number): string => {
    const h = Math.floor(min / 60);
    const m = min % 60;
    if (h === 0) return t("common.duration.minutesShort", { count: m });
    return t("common.duration.hourMinute", { hours: h, minutes: m });
  };

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

  const categoryRanks = useMemo<RankedItem[]>(() => {
    const totals = new Map<string, number>();
    for (const day of days) {
      for (const seg of day.segments) {
        totals.set(
          seg.categoryId,
          (totals.get(seg.categoryId) ?? 0) + seg.minutes,
        );
      }
    }
    const topAppsByCat = new Map<string, string[]>();
    for (const a of apps) {
      if (!a.categoryId) continue;
      const list = topAppsByCat.get(a.categoryId) ?? [];
      list.push(a.iconProcess);
      topAppsByCat.set(a.categoryId, list);
    }
    return categories
      .map((c) => ({
        id: c.id,
        name: displayCategoryName(c, t),
        color: c.color,
        minutes: totals.get(c.id) ?? 0,
        extras: (
          <AppStack
            apps={topAppsByCat.get(c.id) ?? []}
            fallbackColor={c.color}
          />
        ),
      }))
      .filter((c) => c.minutes > 0)
      .sort((a, b) => b.minutes - a.minutes);
  }, [days, apps, categories, t]);

  const appRanks = useMemo<RankedItem[]>(() => {
    return apps.map((a) => {
      const cat = getCategory(a.categoryId);
      const color = cat?.color ?? "#94a3b8";
      return {
        id: a.process,
        name: displayAppName(a.process),
        subtitle: cat ? displayCategoryName(cat, t) : undefined,
        color,
        minutes: a.minutes,
        leading: <AppIcon processName={a.iconProcess} fallbackColor={color} />,
      };
    });
  }, [apps, getCategory, t]);

  const frameRef = useRef<HTMLDivElement>(null);
  const [delta, setDelta] = useState(0);
  const [transitioning, setTransitioning] = useState(false);
  const { ref: prevBtnRef } = useMouseGlow<HTMLButtonElement>();
  const { ref: pillRef } = useMouseGlow<HTMLButtonElement>();
  const { ref: nextBtnRef } = useMouseGlow<HTMLButtonElement>();

  const canGoForward = offset < 0;

  const commit = (direction: -1 | 1) => {
    if (transitioning) return;
    if (direction === 1 && !canGoForward) return;
    const width = frameRef.current?.clientWidth ?? 0;
    setTransitioning(true);
    setDelta(direction === -1 ? width : -width);
    window.setTimeout(() => {
      setTransitioning(false);
      setOffset((o) => o + direction);
      setDelta(0);
    }, SWIPE_DURATION);
  };

  const jumpToThis = () => {
    if (transitioning || offset === 0) return;
    setOffset(0);
  };

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

      <section className={styles.card}>
        <header className={styles.cardHead}>
          <h2 className={styles.cardTitle}>{t("month.chart.title")}</h2>

          <div className={styles.headRight}>
            <DevicePicker />

          <div className={styles.dayNav}>
            <button
              ref={prevBtnRef}
              type="button"
              className={`${styles.navBtn} glow`}
              onClick={() => commit(-1)}
              disabled={transitioning}
              aria-label={t("month.monthNav.prev")}
              title={t("month.monthNav.prev")}
            >
              <ChevronLeft size={14} strokeWidth={1.75} />
            </button>

            <button
              ref={pillRef}
              type="button"
              className={`${styles.dayPill} ${offset !== 0 ? styles.dayPillClickable : ""} glow`}
              onClick={jumpToThis}
              disabled={offset === 0 || transitioning}
              title={offset === 0 ? undefined : t("month.monthNav.backToThisMonth")}
            >
              {monthPillLabel(offset)}
            </button>

            <button
              ref={nextBtnRef}
              type="button"
              className={`${styles.navBtn} glow`}
              onClick={() => commit(1)}
              disabled={!canGoForward || transitioning}
              aria-label={t("month.monthNav.next")}
              title={t("month.monthNav.next")}
            >
              <ChevronRight size={14} strokeWidth={1.75} />
            </button>
          </div>
          </div>
        </header>

        <div className={styles.swipeFrame} ref={frameRef}>
          <div
            className={`${styles.swipeTrack} ${transitioning ? styles.swipeAnimated : ""}`}
            style={{ transform: `translate3d(calc(-100% + ${delta}px), 0, 0)` }}
          >
            {[offset - 1, offset, offset + 1].map((o, idx) => {
              const slideDays = idx === 1 ? days : getMonth(o).days;
              return (
                <div className={styles.slide} key={o}>
                  <DailyBarChart
                    days={slideDays}
                    xLabel={buildXLabel(slideDays)}
                  />
                </div>
              );
            })}
          </div>
        </div>

        <Legend />
      </section>

      <div className={styles.ranks}>
        <section className={styles.card}>
          <header className={styles.cardHead}>
            <h2 className={styles.cardTitle}>{t("month.ranks.topApps")}</h2>
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

function Legend() {
  const { t } = useTranslation();
  const { categories } = useCategories();
  return (
    <div className={styles.legend}>
      {categories.map((c) => (
        <span key={c.id} className={styles.legendItem}>
          <span
            className={styles.legendDot}
            style={{ background: c.color }}
            aria-hidden
          />
          {displayCategoryName(c, t)}
        </span>
      ))}
    </div>
  );
}

function EmptyHint() {
  const { t } = useTranslation();
  return <div className={styles.empty}>{t("common.empty")}</div>;
}
