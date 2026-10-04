import { useEffect, useMemo, useState, type CSSProperties, type PointerEvent } from "react";
import { useTranslation } from "react-i18next";
import { ZoomOut } from "lucide-react";
import { useDurationFormatter } from "../../utils/duration";
import { useIsDark } from "../../hooks/useTheme";
import { adjustCategoryColor } from "../../utils/categoryColor";
import {
  buildHeatmap,
  clipRange,
  LEVEL_MIX,
  parseDayKey,
  type DayRange,
  type HeatCell,
} from "./heatmapGrid";
import styles from "./Heatmap.module.css";

interface HeatmapProps {
  /** 画出来的这段：一整年，或者拖出来的一段 */
  range: DayRange;
  /** 日期键 → 秒数 */
  secsByDay: Map<string, number>;
  /** 有记录的第一天和今天 */
  from: string;
  to: string;
  color: string;
  selectedKey: string | null;
  zoomed: boolean;
  /** 点有时间的格子；再点同一格由调用方决定是否取消 */
  onSelect: (key: string) => void;
  onZoom: (range: DayRange) => void;
  onResetZoom: () => void;
}

/** 从事件落点往上找格子的日期键；空白处为 null */
function keyAt(e: PointerEvent<HTMLElement>): string | null {
  return (e.target as HTMLElement).closest<HTMLElement>("[data-key]")?.dataset.key ?? null;
}

/**
 * 每日热力图：每列一周，颜色深浅按画出来的这段里最忙那天分档。
 * 按住拖过几格就只画这几天（格子跟着变大）；点一格选中那一天。
 */
export function Heatmap({
  range,
  secsByDay,
  from,
  to,
  color,
  selectedKey,
  zoomed,
  onSelect,
  onZoom,
  onResetZoom,
}: HeatmapProps) {
  const { t, i18n } = useTranslation();
  const fmtHM = useDurationFormatter();
  const isDark = useIsDark();
  const [drag, setDrag] = useState<{ start: string; end: string } | null>(null);

  const { weeks, monthCols } = useMemo(
    () => buildHeatmap(range, secsByDay, from, to),
    [range, secsByDay, from, to],
  );

  // 松手可能在热力图外面，所以挂在 window 上
  useEffect(() => {
    if (!drag) return;
    const onUp = () => {
      setDrag(null);
      const lo = drag.start < drag.end ? drag.start : drag.end;
      const hi = drag.start < drag.end ? drag.end : drag.start;
      if (lo === hi) {
        if ((secsByDay.get(lo) ?? 0) > 0 || lo === selectedKey) onSelect(lo);
        return;
      }
      // 拖到第一条记录之前或今天之后的格子，只取有记录的那部分
      const picked = clipRange({ from: lo, to: hi }, { from, to });
      if (picked) onZoom(picked);
    };
    window.addEventListener("pointerup", onUp);
    return () => window.removeEventListener("pointerup", onUp);
  }, [drag, secsByDay, selectedKey, from, to, onSelect, onZoom]);

  const monthFmt = useMemo(
    () => new Intl.DateTimeFormat(i18n.language, { month: "short" }),
    [i18n.language],
  );
  const dateFmt = useMemo(
    () =>
      new Intl.DateTimeFormat(i18n.language, {
        year: "numeric",
        month: "short",
        day: "numeric",
        weekday: "short",
      }),
    [i18n.language],
  );
  // 行头只写周一、周四、周日；2024-01-01 是周一
  const weekdayLabels = useMemo(() => {
    const fmt = new Intl.DateTimeFormat(i18n.language, { weekday: "short" });
    return Array.from({ length: 7 }, (_, i) =>
      i % 3 === 0 ? fmt.format(new Date(2024, 0, 1 + i)) : "",
    );
  }, [i18n.language]);

  const base = adjustCategoryColor(color, isDark);
  // 0 档不给，用 CSS 里的灰色
  const fill = (level: number) =>
    level === 0 ? undefined : `color-mix(in oklab, ${base} ${LEVEL_MIX[level]}%, transparent)`;
  // 格子右边、下边那条缝上画什么线：跟旁边的格子不是同一个月，画月份实线；
  // 两边都没有数据，画格子虚线；有数据的地方方块之间的缝就是分隔，不画
  const edges = (col: number, row: number, cell: HeatCell): CSSProperties => {
    const lineTo = (other: HeatCell | null | undefined) => {
      if (!other) return null;
      if (other.key.slice(0, 7) !== cell.key.slice(0, 7)) {
        return { color: "var(--month-line)", style: "solid" };
      }
      return cell.outside && other.outside ? { color: "var(--grid-line)", style: "dashed" } : null;
    };
    const right = lineTo(weeks[col + 1]?.[row]);
    const below = lineTo(weeks[col][row + 1]);
    return {
      "--right-line": right?.color,
      "--right-style": right?.style,
      "--bottom-line": below?.color,
      "--bottom-style": below?.style,
    } as CSSProperties;
  };
  const columns = { gridTemplateColumns: `repeat(${weeks.length}, minmax(0, 32px))` };
  const dragLo = drag && (drag.start < drag.end ? drag.start : drag.end);
  const dragHi = drag && (drag.start < drag.end ? drag.end : drag.start);

  return (
    <div className={styles.wrap}>
      <div className={styles.body}>
        <div className={styles.months} style={columns}>
          {monthCols.map(({ date, col }) => (
            <span key={col} style={{ gridColumnStart: col + 1 }}>
              {monthFmt.format(date)}
            </span>
          ))}
        </div>
        <div className={styles.weekdays}>
          {weekdayLabels.map((label, i) => (
            <span key={i}>{label}</span>
          ))}
        </div>
        <div
          className={styles.grid}
          style={columns}
          onPointerDown={(e) => {
            const key = e.button === 0 ? keyAt(e) : null;
            if (key === null) return;
            e.preventDefault();
            setDrag({ start: key, end: key });
          }}
          onPointerMove={(e) => {
            const key = drag ? keyAt(e) : null;
            if (drag && key !== null && key !== drag.end) {
              setDrag({ start: drag.start, end: key });
            }
          }}
        >
          {weeks.flatMap((week, col) =>
            week.map((cell, row) => {
              const pos = `${col}-${row}`;
              if (!cell) return <span key={pos} className={styles.blank} />;
              const inDrag =
                dragLo !== null && dragHi !== null && cell.key >= dragLo && cell.key <= dragHi;
              if (cell.outside) {
                return (
                  <span
                    key={pos}
                    data-key={cell.key}
                    className={`${styles.blank} ${inDrag ? styles.inDrag : ""}`}
                    style={edges(col, row, cell)}
                  />
                );
              }
              const label = t("history.heatmap.cell", {
                date: dateFmt.format(parseDayKey(cell.key)),
                duration: fmtHM(Math.round(cell.secs / 60)),
              });
              const selected = cell.key === selectedKey;
              const clickable = cell.secs > 0 || selected;
              return (
                <button
                  key={pos}
                  type="button"
                  data-key={cell.key}
                  className={`${styles.cell} ${selected ? styles.cellSelected : ""} ${inDrag ? styles.inDrag : ""}`}
                  style={{ background: fill(cell.level), ...edges(col, row, cell) }}
                  // 鼠标点击在松手时按拖动处理；这里只接键盘的 Enter / 空格（detail 为 0）
                  onClick={(e) => {
                    if (e.detail === 0 && clickable) onSelect(cell.key);
                  }}
                  aria-disabled={!clickable}
                  aria-pressed={selected}
                  aria-label={label}
                  title={label}
                />
              );
            }),
          )}
        </div>
      </div>

      <div className={styles.footer}>
        {zoomed ? (
          <button type="button" className={styles.resetBtn} onClick={onResetZoom}>
            <ZoomOut size={13} strokeWidth={2} aria-hidden />
            {t("history.chart.resetZoom")}
          </button>
        ) : (
          <span>{t("history.chart.hint")}</span>
        )}
        <span className={styles.legend}>
          <span>{t("history.heatmap.less")}</span>
          {LEVEL_MIX.map((_, level) => (
            <span
              key={level}
              className={styles.legendCell}
              style={{ background: fill(level) }}
            />
          ))}
          <span>{t("history.heatmap.more")}</span>
        </span>
      </div>
    </div>
  );
}
