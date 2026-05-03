import { useMemo, useRef, useState } from "react";
import { ChevronLeft, ChevronRight } from "lucide-react";
import { DEFAULT_CATEGORIES, getCategory } from "../../config/categories";
import { DevicePicker } from "../../components/DevicePicker/DevicePicker";
import { HourlyChart } from "./HourlyChart";
import { RankedList, type RankedItem } from "./RankedList";
import {
  MOCK_HOURS,
  MOCK_TOP_APPS,
  MOCK_WORK_HOURS,
  type HourSlot,
  type AppUsage,
} from "./mockData";
import styles from "./TodayPage.module.css";

const SWIPE_DURATION = 420;

function fmtHM(min: number): string {
  const h = Math.floor(min / 60);
  const m = min % 60;
  if (h === 0) return `${m} 分钟`;
  return `${h} 小时 ${m} 分`;
}

function fmtDate(d: Date): string {
  return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}-${String(d.getDate()).padStart(2, "0")}`;
}

function dayLabel(offset: number): string {
  if (offset === 0) return "今天";
  if (offset === -1) return "昨天";
  if (offset < -1) return `${-offset} 天前`;
  return `${offset} 天后`;
}

function dateForOffset(offset: number): Date {
  const d = new Date();
  d.setDate(d.getDate() + offset);
  return d;
}

function getDayHours(offset: number): HourSlot[] {
  if (offset > 0) {
    return MOCK_HOURS.map((s) => ({ hour: s.hour, segments: [] }));
  }
  if (offset === 0) return MOCK_HOURS;
  const factor = 0.45 + (Math.abs(offset) % 5) * 0.14;
  return MOCK_HOURS.map((s) => ({
    hour: s.hour,
    segments: s.segments.map((seg) => ({
      ...seg,
      minutes: Math.max(0, Math.round(seg.minutes * factor)),
    })),
  }));
}

function getDayApps(offset: number): AppUsage[] {
  if (offset > 0) return [];
  if (offset === 0) return MOCK_TOP_APPS;
  const factor = 0.45 + (Math.abs(offset) % 5) * 0.14;
  return MOCK_TOP_APPS.map((a) => ({
    ...a,
    minutes: Math.max(0, Math.round(a.minutes * factor)),
  }));
}

export default function TodayPage() {
  const [offset, setOffset] = useState(0);

  const hours = useMemo(() => getDayHours(offset), [offset]);
  const apps = useMemo(() => getDayApps(offset), [offset]);

  const totalMinutes = useMemo(
    () =>
      hours.reduce(
        (sum, h) => sum + h.segments.reduce((s, x) => s + x.minutes, 0),
        0,
      ),
    [hours],
  );

  const categoryRanks = useMemo<RankedItem[]>(() => {
    const totals = new Map<string, number>();
    for (const slot of hours) {
      for (const seg of slot.segments) {
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
  }, [hours]);

  const appRanks = useMemo<RankedItem[]>(() => {
    return apps.map((a) => {
      const cat = getCategory(a.categoryId);
      return {
        id: a.process,
        name: a.process,
        subtitle: cat?.name,
        color: cat?.color ?? "#94a3b8",
        minutes: a.minutes,
      };
    });
  }, [apps]);

  // —— 滑动动画状态 ——
  const frameRef = useRef<HTMLDivElement>(null);
  const [delta, setDelta] = useState(0);
  const [transitioning, setTransitioning] = useState(false);

  const canGoForward = offset < 0;

  /** 切到目标方向：先动画到边界，再无过渡复位 */
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

  const jumpToToday = () => {
    if (transitioning || offset === 0) return;
    setOffset(0);
  };

  return (
    <div className={styles.page}>
      <header className={styles.header}>
        <h1 className={styles.title}>今日总览</h1>
        <p className={styles.meta}>
          {fmtDate(dateForOffset(offset))} · 已采集 {fmtHM(totalMinutes)}
        </p>
      </header>

      <section className={styles.card}>
        <header className={styles.cardHead}>
          <h2 className={styles.cardTitle}>24 小时活动分布</h2>

          <div className={styles.headRight}>
            <DevicePicker />

          <div className={styles.dayNav}>
            <button
              type="button"
              className={styles.navBtn}
              onClick={() => commit(-1)}
              disabled={transitioning}
              aria-label="前一天"
              title="前一天"
            >
              <ChevronLeft size={14} strokeWidth={1.75} />
            </button>

            <button
              type="button"
              className={`${styles.dayPill} ${offset !== 0 ? styles.dayPillClickable : ""}`}
              onClick={jumpToToday}
              disabled={offset === 0 || transitioning}
              title={offset === 0 ? undefined : "回到今天"}
            >
              {dayLabel(offset)}
            </button>

            <button
              type="button"
              className={styles.navBtn}
              onClick={() => commit(1)}
              disabled={!canGoForward || transitioning}
              aria-label="后一天"
              title="后一天"
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
              <HourlyChart
                hours={getDayHours(offset - 1)}
                workHours={MOCK_WORK_HOURS}
              />
            </div>
            <div className={styles.slide}>
              <HourlyChart hours={hours} workHours={MOCK_WORK_HOURS} />
            </div>
            <div className={styles.slide}>
              <HourlyChart
                hours={getDayHours(offset + 1)}
                workHours={MOCK_WORK_HOURS}
              />
            </div>
          </div>
        </div>

        <Legend hasWorkHours={!!MOCK_WORK_HOURS && MOCK_WORK_HOURS.length > 0} />
      </section>

      <div className={styles.ranks}>
        <section className={styles.card}>
          <header className={styles.cardHead}>
            <h2 className={styles.cardTitle}>最常用应用</h2>
          </header>
          {appRanks.length > 0 ? (
            <RankedList items={appRanks} />
          ) : (
            <EmptyHint />
          )}
        </section>

        <section className={styles.card}>
          <header className={styles.cardHead}>
            <h2 className={styles.cardTitle}>最常用分类</h2>
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

interface LegendProps {
  hasWorkHours: boolean;
}

function Legend({ hasWorkHours }: LegendProps) {
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
      {hasWorkHours && (
        <span className={styles.legendItem}>
          <span className={styles.legendBand} aria-hidden />
          工作时段
        </span>
      )}
    </div>
  );
}

function EmptyHint() {
  return <div className={styles.empty}>暂无数据</div>;
}
