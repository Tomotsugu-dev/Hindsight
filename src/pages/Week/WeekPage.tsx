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
import { useWeekCache } from "../../hooks/useWeekCache";
import { useDeviceFilter } from "../../state/deviceFilter";
import { useMouseGlow } from "../../hooks/useMouseGlow";
import { WeeklyBarChart } from "./WeeklyBarChart";
import { RankedList, type RankedItem } from "../Today/RankedList";
import type { DaySummary } from "../../api/hindsight";
import styles from "./WeekPage.module.css";

const SWIPE_DURATION = 420;

export default function WeekPage() {
  const { t } = useTranslation();
  const [offset, setOffset] = useState(0);
  const { categories, getCategory } = useCategories();
  const { selectedDeviceId } = useDeviceFilter();
  const { get: getWeek } = useWeekCache(offset, selectedDeviceId);

  const { days, apps } = useMemo(() => getWeek(offset), [getWeek, offset]);

  // 时长格式化 —— 复用 common.duration.* 资源
  const fmtHM = (min: number): string => {
    const h = Math.floor(min / 60);
    const m = min % 60;
    if (h === 0) return t("common.duration.minutesShort", { count: m });
    return t("common.duration.hourMinute", { hours: h, minutes: m });
  };

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

  // —— 滑动动画 ——
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

      <section className={styles.card}>
        <header className={styles.cardHead}>
          <h2 className={styles.cardTitle}>{t("week.chart.title")}</h2>

          <div className={styles.headRight}>
            <DevicePicker />

          <div className={styles.dayNav}>
            <button
              ref={prevBtnRef}
              type="button"
              className={`${styles.navBtn} glow`}
              onClick={() => commit(-1)}
              disabled={transitioning}
              aria-label={t("week.weekNav.prev")}
              title={t("week.weekNav.prev")}
            >
              <ChevronLeft size={14} strokeWidth={1.75} />
            </button>

            <button
              ref={pillRef}
              type="button"
              className={`${styles.dayPill} ${offset !== 0 ? styles.dayPillClickable : ""} glow`}
              onClick={jumpToThis}
              disabled={offset === 0 || transitioning}
              title={offset === 0 ? undefined : t("week.weekNav.backToThisWeek")}
            >
              {weekLabel(offset)}
            </button>

            <button
              ref={nextBtnRef}
              type="button"
              className={`${styles.navBtn} glow`}
              onClick={() => commit(1)}
              disabled={!canGoForward || transitioning}
              aria-label={t("week.weekNav.next")}
              title={t("week.weekNav.next")}
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
            <div className={styles.slide}>
              <WeeklyBarChart days={getWeek(offset - 1).days} />
            </div>
            <div className={styles.slide}>
              <WeeklyBarChart days={days} />
            </div>
            <div className={styles.slide}>
              <WeeklyBarChart days={getWeek(offset + 1).days} />
            </div>
          </div>
        </div>

        <Legend />
      </section>

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
