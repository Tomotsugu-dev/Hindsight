import { useMemo } from "react";
import { useTranslation } from "react-i18next";
import { useDurationFormatter } from "../../utils/duration";
import { useIsDark } from "../../hooks/useTheme";
import { adjustCategoryColor } from "../../utils/categoryColor";
import { buildHeatMonth, parseDayKey } from "./heatmapGrid";
import styles from "./MonthCalendar.module.css";

/** 格子里要写字，颜色比热力图淡，最深一档也压得住文字 */
const CALENDAR_MIX = [0, 16, 28, 42, 58];

interface MonthCalendarProps {
  year: number;
  /** 0–11 */
  month: number;
  secsByDay: Map<string, number>;
  /** 有记录的第一天和今天 */
  from: string;
  to: string;
  color: string;
  selectedKey: string | null;
  onSelect: (key: string) => void;
}

/** 一个月的日历：每格写日期和那天的时长，底色按这个月最忙那天分档。 */
export function MonthCalendar({
  year,
  month,
  secsByDay,
  from,
  to,
  color,
  selectedKey,
  onSelect,
}: MonthCalendarProps) {
  const { i18n } = useTranslation();
  const fmtHM = useDurationFormatter();
  const isDark = useIsDark();

  const weeks = useMemo(
    () => buildHeatMonth(year, month, secsByDay, from, to),
    [year, month, secsByDay, from, to],
  );
  // 2024-01-01 是周一
  const weekdays = useMemo(() => {
    const fmt = new Intl.DateTimeFormat(i18n.language, { weekday: "short" });
    return Array.from({ length: 7 }, (_, i) => fmt.format(new Date(2024, 0, 1 + i)));
  }, [i18n.language]);

  const base = adjustCategoryColor(color, isDark);

  return (
    <div className={styles.wrap}>
      <div className={styles.grid}>
        {weekdays.map((w) => (
          <span key={w} className={styles.weekday}>
            {w}
          </span>
        ))}
        {weeks.flatMap((week, row) =>
          week.map((cell, col) => {
            const pos = `${row}-${col}`;
            if (!cell) return <span key={pos} />;
            const date = parseDayKey(cell.key).getDate();
            if (cell.outside) {
              return (
                <span key={pos} className={`${styles.cell} ${styles.cellOutside}`}>
                  <span className={styles.date}>{date}</span>
                </span>
              );
            }
            const selected = cell.key === selectedKey;
            return (
              <button
                key={pos}
                type="button"
                className={`${styles.cell} ${selected ? styles.cellSelected : ""}`}
                style={{
                  background:
                    cell.level > 0
                      ? `color-mix(in oklab, ${base} ${CALENDAR_MIX[cell.level]}%, transparent)`
                      : undefined,
                }}
                disabled={cell.secs === 0 && !selected}
                onClick={() => onSelect(cell.key)}
                aria-pressed={selected}
              >
                <span className={styles.date}>{date}</span>
                {cell.secs > 0 && (
                  <span className={styles.time}>{fmtHM(Math.round(cell.secs / 60))}</span>
                )}
              </button>
            );
          }),
        )}
      </div>
    </div>
  );
}
