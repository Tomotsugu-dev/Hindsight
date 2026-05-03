import { useMemo, useRef, useState } from "react";
import { ChevronLeft, ChevronRight } from "lucide-react";
import { useCategories } from "../../state/categories";
import { AppIcon } from "../../components/AppIcon/AppIcon";
import { DevicePicker } from "../../components/DevicePicker/DevicePicker";
import { ScrollBox } from "../../components/ScrollBox/ScrollBox";
import { displayAppName } from "../../utils/displayName";
import { useMonthCache } from "../../hooks/useMonthCache";
import { DailyBarChart } from "../Week/DailyBarChart";
import { RankedList, type RankedItem } from "../Today/RankedList";
import type { DaySummary } from "../../api/hindsight";
import styles from "./MonthPage.module.css";

const SWIPE_DURATION = 420;

function fmtHM(min: number): string {
  const h = Math.floor(min / 60);
  const m = min % 60;
  if (h === 0) return `${m} 分`;
  if (m === 0) return `${h} 小时`;
  return `${h} 小时 ${m} 分`;
}

function monthLabel(offset: number): string {
  if (offset === 0) return "本月";
  if (offset === -1) return "上月";
  if (offset < -1) return `${-offset} 月前`;
  return `${offset} 月后`;
}

function fmtMonth(days: DaySummary[], offset: number): string {
  if (days.length > 0) {
    const d = days[0].date;
    return `${d.getFullYear()}年${d.getMonth() + 1}月`;
  }
  const today = new Date();
  const d = new Date(today.getFullYear(), today.getMonth() + offset, 1);
  return `${d.getFullYear()}年${d.getMonth() + 1}月`;
}

export default function MonthPage() {
  const [offset, setOffset] = useState(0);
  const { categories, getCategory } = useCategories();
  const { get: getMonth } = useMonthCache(offset);

  const { days, apps } = useMemo(() => getMonth(offset), [getMonth, offset]);

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
    return categories
      .map((c) => ({
        id: c.id,
        name: c.name,
        color: c.color,
        minutes: totals.get(c.id) ?? 0,
      }))
      .filter((c) => c.minutes > 0)
      .sort((a, b) => b.minutes - a.minutes);
  }, [days, categories]);

  const appRanks = useMemo<RankedItem[]>(() => {
    return apps.map((a) => {
      const cat = getCategory(a.categoryId);
      const color = cat?.color ?? "#94a3b8";
      return {
        id: a.process,
        name: displayAppName(a.process),
        subtitle: cat?.name,
        color,
        minutes: a.minutes,
        leading: <AppIcon processName={a.process} fallbackColor={color} />,
      };
    });
  }, [apps, getCategory]);

  const frameRef = useRef<HTMLDivElement>(null);
  const [delta, setDelta] = useState(0);
  const [transitioning, setTransitioning] = useState(false);

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
        <h1 className={styles.title}>月统计</h1>
        <p className={styles.meta}>
          {fmtMonth(days, offset)} · 共 {fmtHM(totalMinutes)} · 日均 {fmtHM(Math.round(avgPerDay))}
        </p>
      </header>

      <section className={styles.card}>
        <header className={styles.cardHead}>
          <h2 className={styles.cardTitle}>每日活动分布</h2>

          <div className={styles.headRight}>
            <DevicePicker />

          <div className={styles.dayNav}>
            <button
              type="button"
              className={styles.navBtn}
              onClick={() => commit(-1)}
              disabled={transitioning}
              aria-label="前一月"
              title="前一月"
            >
              <ChevronLeft size={14} strokeWidth={1.75} />
            </button>

            <button
              type="button"
              className={`${styles.dayPill} ${offset !== 0 ? styles.dayPillClickable : ""}`}
              onClick={jumpToThis}
              disabled={offset === 0 || transitioning}
              title={offset === 0 ? undefined : "回到本月"}
            >
              {monthLabel(offset)}
            </button>

            <button
              type="button"
              className={styles.navBtn}
              onClick={() => commit(1)}
              disabled={!canGoForward || transitioning}
              aria-label="后一月"
              title="后一月"
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
            <h2 className={styles.cardTitle}>本月最常用应用</h2>
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
            <h2 className={styles.cardTitle}>本月最常用分类</h2>
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
          {c.name}
        </span>
      ))}
    </div>
  );
}

function EmptyHint() {
  return <div className={styles.empty}>暂无数据</div>;
}
