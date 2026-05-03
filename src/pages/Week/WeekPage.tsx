import { useMemo, useRef, useState } from "react";
import { ChevronLeft, ChevronRight } from "lucide-react";
import { DEFAULT_CATEGORIES, getCategory } from "../../config/categories";
import { DevicePicker } from "../../components/DevicePicker/DevicePicker";
import { WeeklyBarChart } from "./WeeklyBarChart";
import { RankedList, type RankedItem } from "../Today/RankedList";
import { getWeekDays, getWeekApps, type DaySummary } from "./mockData";
import styles from "./WeekPage.module.css";

const SWIPE_DURATION = 420;

function fmtHM(min: number): string {
  const h = Math.floor(min / 60);
  const m = min % 60;
  if (h === 0) return `${m} 分`;
  if (m === 0) return `${h} 小时`;
  return `${h} 小时 ${m} 分`;
}

function fmtRange(days: DaySummary[]): string {
  if (days.length === 0) return "";
  const first = days[0].date;
  const last = days[days.length - 1].date;
  const sameMonth = first.getMonth() === last.getMonth();
  if (sameMonth) {
    return `${first.getMonth() + 1}月${first.getDate()}日 — ${last.getDate()}日`;
  }
  return `${first.getMonth() + 1}月${first.getDate()}日 — ${last.getMonth() + 1}月${last.getDate()}日`;
}

function weekLabel(offset: number): string {
  if (offset === 0) return "本周";
  if (offset === -1) return "上周";
  if (offset < -1) return `${-offset} 周前`;
  return `${offset} 周后`;
}

export default function WeekPage() {
  const [offset, setOffset] = useState(0);

  const days = useMemo(() => getWeekDays(offset), [offset]);

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
    return DEFAULT_CATEGORIES
      .map((c) => ({
        id: c.id,
        name: c.name,
        color: c.color,
        minutes: totals.get(c.id) ?? 0,
      }))
      .filter((c) => c.minutes > 0)
      .sort((a, b) => b.minutes - a.minutes);
  }, [days]);

  const appRanks = useMemo<RankedItem[]>(() => {
    return getWeekApps(offset).map((a) => {
      const cat = getCategory(a.categoryId);
      return {
        id: a.process,
        name: a.process,
        subtitle: cat?.name,
        color: cat?.color ?? "#94a3b8",
        minutes: a.minutes,
      };
    });
  }, [offset]);

  // —— 滑动动画 ——
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

  return (
    <div className={styles.page}>
      <header className={styles.header}>
        <h1 className={styles.title}>周统计</h1>
        <p className={styles.meta}>
          {fmtRange(days)} · 共 {fmtHM(totalMinutes)} · 日均 {fmtHM(Math.round(avgPerDay))}
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
              aria-label="前一周"
              title="前一周"
            >
              <ChevronLeft size={14} strokeWidth={1.75} />
            </button>

            <button
              type="button"
              className={`${styles.dayPill} ${offset !== 0 ? styles.dayPillClickable : ""}`}
              onClick={jumpToThis}
              disabled={offset === 0 || transitioning}
              title={offset === 0 ? undefined : "回到本周"}
            >
              {weekLabel(offset)}
            </button>

            <button
              type="button"
              className={styles.navBtn}
              onClick={() => commit(1)}
              disabled={!canGoForward || transitioning}
              aria-label="后一周"
              title="后一周"
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
              <WeeklyBarChart days={getWeekDays(offset - 1)} />
            </div>
            <div className={styles.slide}>
              <WeeklyBarChart days={days} />
            </div>
            <div className={styles.slide}>
              <WeeklyBarChart days={getWeekDays(offset + 1)} />
            </div>
          </div>
        </div>

        <Legend />
      </section>

      <div className={styles.ranks}>
        <section className={styles.card}>
          <header className={styles.cardHead}>
            <h2 className={styles.cardTitle}>本周最常用应用</h2>
          </header>
          {appRanks.length > 0 ? (
            <RankedList items={appRanks} />
          ) : (
            <EmptyHint />
          )}
        </section>

        <section className={styles.card}>
          <header className={styles.cardHead}>
            <h2 className={styles.cardTitle}>本周最常用分类</h2>
          </header>
          {categoryRanks.length > 0 ? (
            <RankedList items={categoryRanks} />
          ) : (
            <EmptyHint />
          )}
        </section>
      </div>
    </div>
  );
}

function Legend() {
  return (
    <div className={styles.legend}>
      {DEFAULT_CATEGORIES.map((c) => (
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
