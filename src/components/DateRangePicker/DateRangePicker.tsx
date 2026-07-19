import { useLayoutEffect, useState } from "react";
import { CalendarDays, ChevronLeft, ChevronRight } from "lucide-react";
import { useTranslation } from "react-i18next";
import { usePicker } from "../../hooks/usePicker";
import styles from "./DateRangePicker.module.css";

/** 日期范围选择器(自绘日历,替代 `<input type="date">`——原生弹出日历是浏览器
 *  控件,样式/星期起点完全不受控,与应用设计语言脱节)。
 *
 *  交互(定稿:两框式,靠结构而非分隔符消除歧义):
 *  - 起始/终止各一个框,中间静态"～";点哪个框就在日历里选哪个日期,
 *    选完起始自动切到终止(机票预订式动线),选完终止收起面板;
 *  - 选择后若 起 > 止 自动交换,永不产生非法区间;
 *  - 周一开头(与周统计口径一致);星期/月份标题走 Intl 按界面语言本地化;
 *  - `max` 之后的天禁选(未来无数据);今天带圆点标记;
 *  - 开合/外点/Esc 复用 usePicker,面板视觉对齐 SimplePicker。 */

interface Props {
  /** "YYYY-MM-DD" */
  start: string;
  end: string;
  /** 可选的最晚日期(含),通常是今天 */
  max: string;
  disabled?: boolean;
  onChange: (start: string, end: string) => void;
}

type Field = "start" | "end";

function fmtLocal(d: Date): string {
  const y = d.getFullYear();
  const m = String(d.getMonth() + 1).padStart(2, "0");
  const day = String(d.getDate()).padStart(2, "0");
  return `${y}-${m}-${day}`;
}

function parseLocal(s: string): Date {
  const [y, m, d] = s.split("-").map((v) => parseInt(v, 10));
  return new Date(y, m - 1, d);
}

/** 该月日历页的 42 个格子(6 行 × 周一开头)。 */
function calendarCells(viewMonth: Date): Date[] {
  const first = new Date(viewMonth.getFullYear(), viewMonth.getMonth(), 1);
  const day = first.getDay(); // 0=周日
  const mondayOffset = day === 0 ? 6 : day - 1;
  const cells: Date[] = [];
  for (let i = 0; i < 42; i++) {
    cells.push(
      new Date(first.getFullYear(), first.getMonth(), first.getDate() - mondayOffset + i),
    );
  }
  return cells;
}

export function DateRangePicker({ start, end, max, disabled, onChange }: Props) {
  const { t, i18n } = useTranslation();
  const { open, wrapRef, openMenu, close } = usePicker();
  /** 日历当前服务于哪个框 */
  const [active, setActive] = useState<Field>("start");
  // 面板当前显示的月份(该月 1 号)
  const [viewMonth, setViewMonth] = useState(() => {
    const e = parseLocal(end || max);
    return new Date(e.getFullYear(), e.getMonth(), 1);
  });
  const [direction, setDirection] = useState<"down" | "up">("down");

  // 打开/切换目标框时:日历跳到该框日期所在月;按视口余量决定展开方向(~330px)
  useLayoutEffect(() => {
    if (!open) return;
    const target = parseLocal((active === "start" ? start : end) || max);
    setViewMonth(new Date(target.getFullYear(), target.getMonth(), 1));
    if (wrapRef.current) {
      const rect = wrapRef.current.getBoundingClientRect();
      const spaceBelow = window.innerHeight - rect.bottom;
      setDirection(spaceBelow >= 330 || spaceBelow >= rect.top ? "down" : "up");
    }
    // start/end 的常规变化不重新定位——只在开面板/切框瞬间对齐一次
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open, active, wrapRef]);

  const todayStr = fmtLocal(new Date());
  const monthTitle = new Intl.DateTimeFormat(i18n.language, {
    year: "numeric",
    month: "long",
  }).format(viewMonth);
  // 周一开头的星期表头(2024-01-01 恰是周一,借它生成本地化窄名)
  const weekdayFmt = new Intl.DateTimeFormat(i18n.language, { weekday: "narrow" });
  const weekdays = Array.from({ length: 7 }, (_, i) => weekdayFmt.format(new Date(2024, 0, 1 + i)));

  const nextMonthFirst = new Date(viewMonth.getFullYear(), viewMonth.getMonth() + 1, 1);
  const nextDisabled = fmtLocal(nextMonthFirst) > max;

  const clickField = (field: Field) => {
    if (disabled) return;
    setActive(field);
    if (!open) openMenu();
  };

  const pickDay = (dstr: string) => {
    let [lo, hi] = active === "start" ? [dstr, end] : [start, dstr];
    if (lo > hi) [lo, hi] = [hi, lo];
    onChange(lo, hi);
    if (active === "start") {
      setActive("end"); // 选完起始自动进入终止选择,面板保持打开
    } else {
      close();
    }
  };

  const fieldBtn = (field: Field, value: string) => (
    <button
      type="button"
      className={`${styles.fieldBtn} ${open && active === field ? styles.fieldBtnActive : ""}`}
      onClick={() => clickField(field)}
      disabled={disabled}
      aria-haspopup="dialog"
      aria-expanded={open && active === field}
      aria-label={t(field === "start" ? "common.dateRange.startAria" : "common.dateRange.endAria")}
    >
      <CalendarDays size={13} strokeWidth={1.9} className={styles.fieldIcon} />
      {value}
    </button>
  );

  return (
    <div className={styles.wrap} ref={wrapRef}>
      <div className={styles.fields}>
        {fieldBtn("start", start)}
        <span className={styles.sep} aria-hidden>
          ～
        </span>
        {fieldBtn("end", end)}
      </div>

      {open && (
        <div className={styles.panel} data-direction={direction} role="dialog">
          <div className={styles.head}>
            <button
              type="button"
              className={styles.navBtn}
              onClick={() =>
                setViewMonth(new Date(viewMonth.getFullYear(), viewMonth.getMonth() - 1, 1))
              }
              aria-label={t("common.dateRange.prevMonth")}
            >
              <ChevronLeft size={14} strokeWidth={2} />
            </button>
            <span className={styles.monthTitle}>{monthTitle}</span>
            <button
              type="button"
              className={styles.navBtn}
              onClick={() => setViewMonth(nextMonthFirst)}
              disabled={nextDisabled}
              aria-label={t("common.dateRange.nextMonth")}
            >
              <ChevronRight size={14} strokeWidth={2} />
            </button>
          </div>

          {/* role=grid:日历的正确语义 */}
          <div className={styles.grid} role="grid">
            {weekdays.map((w, i) => (
              <span key={`h${i}`} className={styles.weekHead}>
                {w}
              </span>
            ))}
            {calendarCells(viewMonth).map((d) => {
              const dstr = fmtLocal(d);
              const outside = d.getMonth() !== viewMonth.getMonth();
              const over = dstr > max;
              const isStart = dstr === start;
              const isEnd = dstr === end;
              const inRange = dstr > start && dstr < end;
              return (
                <button
                  key={dstr}
                  type="button"
                  className={[
                    styles.cell,
                    outside ? styles.cellOutside : "",
                    isStart || isEnd ? styles.cellEndpoint : "",
                    inRange ? styles.cellInRange : "",
                    dstr === todayStr ? styles.cellToday : "",
                  ]
                    .filter(Boolean)
                    .join(" ")}
                  disabled={over}
                  onClick={() => pickDay(dstr)}
                >
                  {d.getDate()}
                </button>
              );
            })}
          </div>
        </div>
      )}
    </div>
  );
}
