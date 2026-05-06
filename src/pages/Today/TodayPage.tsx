import { useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { ChevronLeft, ChevronRight } from "lucide-react";
import { useCategories } from "../../state/categories";
import { useSettings } from "../../state/settings";
import { DevicePicker } from "../../components/DevicePicker/DevicePicker";
import { AppIcon } from "../../components/AppIcon/AppIcon";
import { AppStack } from "../../components/AppStack/AppStack";
import { ScrollBox } from "../../components/ScrollBox/ScrollBox";
import { displayAppName } from "../../utils/displayName";
import { HourlyChart, type WorkRange } from "./HourlyChart";
import { RankedList, type RankedItem } from "./RankedList";
import { useDayCache } from "../../hooks/useDayCache";
import { useDeviceFilter } from "../../state/deviceFilter";
import { useMouseGlow } from "../../hooks/useMouseGlow";
import styles from "./TodayPage.module.css";

const SWIPE_DURATION = 420;

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
  const [offset, setOffset] = useState(0);
  const { selectedDeviceId } = useDeviceFilter();
  const { get: getDay } = useDayCache(offset, selectedDeviceId);
  const { categories, getCategory } = useCategories();
  const { settings } = useSettings();

  // 把分钟数格式化成本地化的时长文案 —— 依赖 i18n，因此放在组件内
  const fmtHM = (min: number): string => {
    const h = Math.floor(min / 60);
    const m = min % 60;
    if (h === 0) return t("common.duration.minutesShort", { count: m });
    return t("common.duration.hourMinute", { hours: h, minutes: m });
  };

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
    const topAppsByCat = new Map<string, string[]>();
    for (const a of apps) {
      if (!a.categoryId) continue;
      const list = topAppsByCat.get(a.categoryId) ?? [];
      // AppStack 拿这串去查图标，必须用 iconProcess（合并组里的稳定代表名）；
      // a.process 是组的 display_name，可能跟 app_icons 表里的 key 不一致。
      list.push(a.iconProcess);
      topAppsByCat.set(a.categoryId, list);
    }
    return categories
      .map((c) => ({
        id: c.id,
        name: c.name,
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
  }, [hours, apps, categories]);

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
        leading: <AppIcon processName={a.iconProcess} fallbackColor={color} />,
      };
    });
  }, [apps, getCategory]);

  // —— 滑动动画状态 ——
  const frameRef = useRef<HTMLDivElement>(null);
  const [delta, setDelta] = useState(0);
  const [transitioning, setTransitioning] = useState(false);
  const { ref: prevBtnRef } = useMouseGlow<HTMLButtonElement>();
  const { ref: pillRef } = useMouseGlow<HTMLButtonElement>();
  const { ref: nextBtnRef } = useMouseGlow<HTMLButtonElement>();

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
        <h1 className={styles.title}>{t("today.title")}</h1>
        <p className={styles.meta}>
          {t("today.meta", {
            date: fmtDate(dateForOffset(offset)),
            duration: fmtHM(totalMinutes),
          })}
        </p>
      </header>

      <section className={styles.card}>
        <header className={styles.cardHead}>
          <h2 className={styles.cardTitle}>{t("today.chart.title")}</h2>

          <div className={styles.headRight}>
            <DevicePicker />

          <div className={styles.dayNav}>
            <button
              ref={prevBtnRef}
              type="button"
              className={`${styles.navBtn} glow`}
              onClick={() => commit(-1)}
              disabled={transitioning}
              aria-label={t("today.dayNav.prev")}
              title={t("today.dayNav.prev")}
            >
              <ChevronLeft size={14} strokeWidth={1.75} />
            </button>

            <button
              ref={pillRef}
              type="button"
              className={`${styles.dayPill} ${offset !== 0 ? styles.dayPillClickable : ""} glow`}
              onClick={jumpToToday}
              disabled={offset === 0 || transitioning}
              title={offset === 0 ? undefined : t("today.dayNav.backToToday")}
            >
              {dayLabel(offset)}
            </button>

            <button
              ref={nextBtnRef}
              type="button"
              className={`${styles.navBtn} glow`}
              onClick={() => commit(1)}
              disabled={!canGoForward || transitioning}
              aria-label={t("today.dayNav.next")}
              title={t("today.dayNav.next")}
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
                hours={getDay(offset - 1).hours}
                workHours={workRanges}
              />
            </div>
            <div className={styles.slide}>
              <HourlyChart hours={hours} workHours={workRanges} />
            </div>
            <div className={styles.slide}>
              <HourlyChart
                hours={getDay(offset + 1).hours}
                workHours={workRanges}
              />
            </div>
          </div>
        </div>

        <Legend hasWorkHours={!!workRanges} />
      </section>

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

interface LegendProps {
  hasWorkHours: boolean;
}

function Legend({ hasWorkHours }: LegendProps) {
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
          {c.name}
        </span>
      ))}
      {hasWorkHours && (
        <span className={styles.legendItem}>
          <span className={styles.legendBand} aria-hidden />
          {t("today.legend.workHours")}
        </span>
      )}
    </div>
  );
}

function EmptyHint() {
  const { t } = useTranslation();
  return <div className={styles.empty}>{t("common.empty")}</div>;
}
