import { useContext, useId, useRef, useState, type CSSProperties } from "react";
import { useTranslation } from "react-i18next";
import { RowLabelContext } from "../FormLayout/rowLabelContext";
import styles from "./Slider.module.css";
import { adjacentStop, nearestStop, stopPosition } from "./sliderStops";

interface SliderProps {
  value: number;
  onChange: (next: number) => void;
  min: number;
  max: number;
  step?: number;
  /** 数值显示后缀，如 "秒" / "天" */
  suffix?: string;
  /** Text to show in place of the number and suffix, such as "Forever". Undefined shows the number. */
  valueLabel?: (value: number) => string | undefined;
  /**
   * Values that dragging snaps to, in increasing order, spaced evenly on the track. Typing in the
   * value box still takes any value from min to max. A stop above max, such as "Forever", is
   * reached only by dragging.
   */
  stops?: readonly number[];
}

export function Slider({
  value,
  onChange,
  min,
  max,
  step = 1,
  suffix,
  valueLabel,
  stops,
}: SliderProps) {
  const { t } = useTranslation();
  const label = valueLabel?.(value);
  const id = useId();
  // Row 通过 context 下传可见 label 的 id；不在 Row 内时回落到通用 valueAria
  const rowLabelId = useContext(RowLabelContext);
  const position = stops ? stopPosition(value, stops) : value;
  const trackMin = stops ? 0 : min;
  const trackMax = stops ? stops.length - 1 : max;
  const percent = ((position - trackMin) / (trackMax - trackMin)) * 100;

  const [focused, setFocused] = useState(false);
  const typedRef = useRef("");
  const typeTimerRef = useRef<number | null>(null);

  const maxDigits = String(Math.max(Math.abs(max), Math.abs(min))).length;
  const valStr = String(value);
  const padded = valStr.padStart(maxDigits, "0");
  const visibleStart = maxDigits - valStr.length;

  const clamp = (v: number) => Math.max(min, Math.min(max, v));
  const snap = (v: number) => Math.round((v - min) / step) * step + min;

  const change = (delta: number) => {
    typedRef.current = "";
    // A stop above max stays where it is when raised
    if (value > max && delta > 0) return;
    onChange(clamp(value + delta));
  };

  const moveToStop = (direction: 1 | -1) => {
    if (!stops) return;
    const next = adjacentStop(value, stops, direction);
    if (next !== value) onChange(next);
  };

  const resetTypedAfter = (ms: number) => {
    if (typeTimerRef.current) clearTimeout(typeTimerRef.current);
    typeTimerRef.current = window.setTimeout(() => {
      typedRef.current = "";
    }, ms);
  };

  const acceptDigit = (d: number) => {
    const next = typedRef.current + String(d);
    const numeric = parseInt(next, 10);
    if (numeric > max) {
      // 已超上限：把这位当作新的第一位重来
      typedRef.current = String(d);
      onChange(clamp(d));
    } else {
      typedRef.current = next;
      onChange(clamp(numeric));
    }
    resetTypedAfter(1500);
  };

  return (
    <div className={styles.wrap} data-part="slider">
      <div className={styles.trackWrap}>
        <input
          id={id}
          type="range"
          min={trackMin}
          max={trackMax}
          step={stops ? "any" : step}
          value={position}
          onChange={(e) => {
            const p = Number(e.target.value);
            const next = stops ? nearestStop(p, stops) : p;
            if (next !== value) onChange(next);
          }}
          onKeyDown={(e) => {
            if (!stops) return;
            if (e.key === "ArrowRight" || e.key === "ArrowUp") {
              e.preventDefault();
              moveToStop(1);
            } else if (e.key === "ArrowLeft" || e.key === "ArrowDown") {
              e.preventDefault();
              moveToStop(-1);
            }
          }}
          className={styles.input}
          style={{ "--fill": `${percent}%` } as CSSProperties}
        />
      </div>
      <button
        type="button"
        role="spinbutton"
        aria-valuemin={min}
        aria-valuemax={max}
        aria-valuenow={value}
        aria-valuetext={label}
        aria-label={rowLabelId ? undefined : t("components.slider.valueAria")}
        aria-labelledby={rowLabelId}
        className={`${styles.valueBox} ${focused ? styles.valueBoxFocused : ""}`}
        onMouseDown={(e) => {
          // WebKit does not focus a button on click, so typed digits would go nowhere
          e.preventDefault();
          e.currentTarget.focus();
        }}
        onFocus={() => setFocused(true)}
        onBlur={() => {
          setFocused(false);
          typedRef.current = "";
          if (typeTimerRef.current) clearTimeout(typeTimerRef.current);
          // Align to step on blur. With stops, typing already gives a whole value, and a stop
          // above max must not be pulled down to max.
          if (stops) return;
          const snapped = clamp(snap(value));
          if (snapped !== value) onChange(snapped);
        }}
        onKeyDown={(e) => {
          if (e.key === "ArrowUp") {
            e.preventDefault();
            change(step);
          } else if (e.key === "ArrowDown") {
            e.preventDefault();
            change(-step);
          } else if (/^\d$/.test(e.key)) {
            e.preventDefault();
            acceptDigit(parseInt(e.key, 10));
          } else if (e.key === "Backspace" || e.key === "Delete") {
            e.preventDefault();
            typedRef.current = "";
            onChange(min);
          } else if (e.key === "Enter") {
            e.preventDefault();
            e.currentTarget.blur();
          }
        }}
        onWheel={(e) => {
          if (!focused) return;
          e.preventDefault();
          change(e.deltaY > 0 ? -step : step);
        }}
      >
        {label !== undefined ? (
          <span className={styles.label}>{label}</span>
        ) : (
          <>
            <span className={styles.digits}>
              {padded.split("").map((d, i) => (
                <DigitColumn key={i} digit={Number(d)} hidden={i < visibleStart} />
              ))}
            </span>
            {suffix ? <span className={styles.suffix}>{suffix}</span> : null}
          </>
        )}
      </button>
    </div>
  );
}

function DigitColumn({ digit, hidden }: { digit: number; hidden: boolean }) {
  return (
    <span className={`${styles.column} ${hidden ? styles.columnHidden : ""}`} aria-hidden>
      <span className={styles.strip} style={{ transform: `translateY(-${digit}em)` }}>
        {Array.from({ length: 10 }, (_, i) => (
          <span key={i} className={styles.digit}>
            {i}
          </span>
        ))}
      </span>
    </span>
  );
}
