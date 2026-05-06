import { useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import styles from "./TimeField.module.css";

interface TimeFieldProps {
  /** "HH:mm" */
  value: string;
  onChange: (next: string) => void;
}

function parse(v: string): [number, number] {
  const [h, m] = v.split(":").map((s) => parseInt(s, 10));
  return [isNaN(h) ? 0 : h, isNaN(m) ? 0 : m];
}

function format(h: number, m: number): string {
  return `${String(h).padStart(2, "0")}:${String(m).padStart(2, "0")}`;
}

export function TimeField({ value, onChange }: TimeFieldProps) {
  const { t } = useTranslation();
  const [h, m] = parse(value);
  return (
    <div className={styles.wrap}>
      <SpinPart
        value={h}
        max={23}
        onChange={(next) => onChange(format(next, m))}
        ariaLabel={t("components.timeField.hourAria")}
      />
      <span className={styles.colon}>:</span>
      <SpinPart
        value={m}
        max={59}
        onChange={(next) => onChange(format(h, next))}
        ariaLabel={t("components.timeField.minuteAria")}
      />
      <span className={styles.tooltip} role="tooltip">
        {t("components.timeField.tooltip")}
      </span>
    </div>
  );
}

interface SpinPartProps {
  value: number;
  max: number;
  onChange: (next: number) => void;
  ariaLabel: string;
}

type Direction = "up" | "down" | null;

function SpinPart({ value, max, onChange, ariaLabel }: SpinPartProps) {
  const [focused, setFocused] = useState(false);
  const [direction, setDirection] = useState<Direction>(null);
  const wheelAcc = useRef(0);

  /** 输入数字时用的缓冲区 — 用 ref 避免每次按键触发渲染 */
  const typedRef = useRef("");
  const typeTimerRef = useRef<number | null>(null);

  const change = (delta: number) => {
    const total = max + 1;
    onChange((value + delta + total) % total);
  };

  const dirFromY = (e: React.MouseEvent<HTMLButtonElement>): "up" | "down" => {
    const rect = e.currentTarget.getBoundingClientRect();
    return e.clientY - rect.top < rect.height / 2 ? "up" : "down";
  };

  const resetTypedAfter = (ms: number) => {
    if (typeTimerRef.current) clearTimeout(typeTimerRef.current);
    typeTimerRef.current = window.setTimeout(() => {
      typedRef.current = "";
    }, ms);
  };

  /** 智能解析两位数：第一位若已不可能与下一位组成合法值，立即视为终值 */
  const acceptDigit = (d: number) => {
    if (typedRef.current.length === 0) {
      // 第一位
      if (d * 10 > max) {
        onChange(d);
        typedRef.current = "";
      } else {
        typedRef.current = String(d);
        onChange(d);
      }
    } else {
      // 第二位
      const prev = parseInt(typedRef.current, 10);
      const combined = prev * 10 + d;
      if (combined <= max) {
        onChange(combined);
        typedRef.current = "";
      } else {
        // 组合越界 → 把这位当作新的第一位重来
        if (d * 10 > max) {
          onChange(d);
          typedRef.current = "";
        } else {
          typedRef.current = String(d);
          onChange(d);
        }
      }
    }
    resetTypedAfter(1000);
  };

  return (
    <button
      type="button"
      role="spinbutton"
      aria-label={ariaLabel}
      aria-valuenow={value}
      aria-valuemin={0}
      aria-valuemax={max}
      className={`${styles.part} ${focused ? styles.focused : ""} ${
        direction === "up" ? styles.dirUp : direction === "down" ? styles.dirDown : ""
      }`}
      onFocus={() => setFocused(true)}
      onBlur={() => {
        setFocused(false);
        typedRef.current = "";
        if (typeTimerRef.current) clearTimeout(typeTimerRef.current);
      }}
      onMouseMove={(e) => setDirection(dirFromY(e))}
      onMouseLeave={() => setDirection(null)}
      onClick={(e) => change(dirFromY(e) === "up" ? 1 : -1)}
      onWheel={(e) => {
        e.preventDefault();
        wheelAcc.current += e.deltaY;
        if (Math.abs(wheelAcc.current) >= 30) {
          change(wheelAcc.current > 0 ? -1 : 1);
          wheelAcc.current = 0;
        }
      }}
      onKeyDown={(e) => {
        if (e.key === "ArrowUp") {
          e.preventDefault();
          change(1);
        } else if (e.key === "ArrowDown") {
          e.preventDefault();
          change(-1);
        } else if (/^\d$/.test(e.key)) {
          e.preventDefault();
          acceptDigit(parseInt(e.key, 10));
        } else if (e.key === "Backspace" || e.key === "Delete") {
          e.preventDefault();
          typedRef.current = "";
          onChange(0);
        }
      }}
    >
      <DigitColumn digit={Math.floor(value / 10)} />
      <DigitColumn digit={value % 10} />
    </button>
  );
}

function DigitColumn({ digit }: { digit: number }) {
  return (
    <span className={styles.column} aria-hidden>
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
