import { useEffect, useRef, useState } from "react";
import { Check, ChevronDown } from "lucide-react";
import styles from "./SimplePicker.module.css";

export interface SimplePickerOption<T extends string> {
  value: T;
  label: string;
}

interface SimplePickerProps<T extends string> {
  value: T;
  options: SimplePickerOption<T>[];
  onChange: (next: T) => void;
  disabled?: boolean;
}

/**
 * 通用下拉选择器，跟 DevicePicker 视觉完全一致（去掉了 tile / 设备图标）。
 * 用 string-typed value 做精确匹配，泛型 T 让调用方拿到 onChange 的精确类型。
 */
export function SimplePicker<T extends string>({
  value,
  options,
  onChange,
  disabled,
}: SimplePickerProps<T>) {
  const [open, setOpen] = useState(false);
  const wrapRef = useRef<HTMLDivElement>(null);

  const current = options.find((o) => o.value === value);

  // 点外面关闭
  useEffect(() => {
    if (!open) return;
    const onClick = (e: MouseEvent) => {
      if (wrapRef.current && !wrapRef.current.contains(e.target as Node)) {
        setOpen(false);
      }
    };
    document.addEventListener("mousedown", onClick);
    return () => document.removeEventListener("mousedown", onClick);
  }, [open]);

  return (
    <div className={styles.wrap} ref={wrapRef}>
      <button
        type="button"
        className={`${styles.trigger} ${open ? styles.triggerOpen : ""}`}
        onClick={() => !disabled && setOpen((v) => !v)}
        disabled={disabled}
        aria-haspopup="listbox"
        aria-expanded={open}
      >
        {/* 用 grid 让所有候选 label 占同一格，trigger 宽度跟着最宽的 label 走，
            切换不抖。隐藏的 measure span 只撑宽，可见的 label 居中显示。 */}
        <span className={styles.labelStack}>
          <span className={styles.label}>{current?.label ?? ""}</span>
          {options.map((opt) => (
            <span
              key={opt.value}
              className={styles.labelMeasure}
              aria-hidden
            >
              {opt.label}
            </span>
          ))}
        </span>
        <ChevronDown
          size={12}
          strokeWidth={2}
          className={`${styles.chev} ${open ? styles.chevOpen : ""}`}
        />
      </button>

      {open && (
        <div className={styles.menu} role="listbox">
          {options.map((opt) => (
            <button
              key={opt.value}
              type="button"
              className={`${styles.item} ${
                opt.value === value ? styles.itemChecked : ""
              }`}
              onClick={() => {
                onChange(opt.value);
                setOpen(false);
              }}
              role="option"
              aria-selected={opt.value === value}
            >
              <span className={styles.itemLabel}>{opt.label}</span>
              {opt.value === value && (
                <Check
                  size={13}
                  strokeWidth={2.25}
                  className={styles.itemCheck}
                />
              )}
            </button>
          ))}
        </div>
      )}
    </div>
  );
}
