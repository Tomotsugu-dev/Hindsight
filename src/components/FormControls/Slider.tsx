import { useContext, useId, useRef, useState, type CSSProperties } from "react";
import { useTranslation } from "react-i18next";
import { RowLabelContext } from "../FormLayout/rowLabelContext";
import styles from "./Slider.module.css";

interface SliderProps {
  value: number;
  onChange: (next: number) => void;
  min: number;
  max: number;
  step?: number;
  /** 数值显示后缀，如 "秒" / "天" */
  suffix?: string;
}

export function Slider({ value, onChange, min, max, step = 1, suffix }: SliderProps) {
  const { t } = useTranslation();
  const id = useId();
  // Row 通过 context 下传可见 label 的 id；不在 Row 内时回落到通用 valueAria
  const rowLabelId = useContext(RowLabelContext);
  const percent = ((value - min) / (max - min)) * 100;

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
    onChange(clamp(value + delta));
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
    <div className={styles.wrap}>
      <div className={styles.trackWrap}>
        <input
          id={id}
          type="range"
          min={min}
          max={max}
          step={step}
          value={value}
          onChange={(e) => onChange(Number(e.target.value))}
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
        aria-label={rowLabelId ? undefined : t("components.slider.valueAria")}
        aria-labelledby={rowLabelId}
        className={`${styles.valueBox} ${focused ? styles.valueBoxFocused : ""}`}
        onFocus={() => setFocused(true)}
        onBlur={() => {
          setFocused(false);
          typedRef.current = "";
          if (typeTimerRef.current) clearTimeout(typeTimerRef.current);
          // 失焦时按 step 对齐
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
            (e.currentTarget).blur();
          }
        }}
        onWheel={(e) => {
          if (!focused) return;
          e.preventDefault();
          change(e.deltaY > 0 ? -step : step);
        }}
      >
        <span className={styles.digits}>
          {padded.split("").map((d, i) => (
            <DigitColumn
              key={i}
              digit={Number(d)}
              hidden={i < visibleStart}
            />
          ))}
        </span>
        {suffix ? <span className={styles.suffix}>{suffix}</span> : null}
      </button>
    </div>
  );
}

function DigitColumn({ digit, hidden }: { digit: number; hidden: boolean }) {
  return (
    <span
      className={`${styles.column} ${hidden ? styles.columnHidden : ""}`}
      aria-hidden
    >
      <span
        className={styles.strip}
        style={{ transform: `translateY(-${digit}em)` }}
      >
        {Array.from({ length: 10 }, (_, i) => (
          <span key={i} className={styles.digit}>
            {i}
          </span>
        ))}
      </span>
    </span>
  );
}
