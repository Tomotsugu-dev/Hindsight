import { useId } from "react";
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
  const id = useId();
  const percent = ((value - min) / (max - min)) * 100;

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
          style={{ "--fill": `${percent}%` } as React.CSSProperties}
        />
      </div>
      <div className={styles.valueBox}>
        <span className={styles.valueText}>{value}</span>
        {suffix ? <span className={styles.suffix}>{suffix}</span> : null}
      </div>
    </div>
  );
}
