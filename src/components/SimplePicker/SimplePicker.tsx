import { useLayoutEffect, useState } from "react";
import { Check, ChevronDown } from "lucide-react";
import { usePicker } from "../../hooks/usePicker";
import styles from "./SimplePicker.module.css";

/** 菜单展开方向：下拉默认向下，靠近视口底时自动翻向上。 */
type MenuDirection = "down" | "up";
/** 单条 .item 高度 32 + 4px 上下 padding 估算；6px 是 trigger 到菜单的 gap。 */
const ITEM_HEIGHT = 32;
const MENU_PADDING = 8;
const TRIGGER_GAP = 6;

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
 *
 * 展开/外点关闭/Esc 关闭抽到 [usePicker] hook。
 */
export function SimplePicker<T extends string>({
  value,
  options,
  onChange,
  disabled,
}: SimplePickerProps<T>) {
  const { open, wrapRef, toggle, close } = usePicker();
  const current = options.find((o) => o.value === value);
  const [direction, setDirection] = useState<MenuDirection>("down");

  // 打开时按 trigger 在视口里的位置 + 估算菜单高度，决定向下还是向上展开。
  // 避免窗口最大 720px 高度下，靠底的 picker（如 EngineTab 的 ctxSize）下拉
  // 撑出窗口被裁。useLayoutEffect 在 paint 前定方向，避免一帧闪现。
  useLayoutEffect(() => {
    if (!open || !wrapRef.current) return;
    const rect = wrapRef.current.getBoundingClientRect();
    const estMenuHeight = options.length * ITEM_HEIGHT + MENU_PADDING + TRIGGER_GAP;
    const spaceBelow = window.innerHeight - rect.bottom;
    const spaceAbove = rect.top;
    // 下方装得下 → 下拉；下方不够但上方更宽敞 → 上拉
    setDirection(spaceBelow >= estMenuHeight || spaceBelow >= spaceAbove ? "down" : "up");
  }, [open, options.length, wrapRef]);

  return (
    <div className={styles.wrap} ref={wrapRef}>
      <button
        type="button"
        className={`${styles.trigger} ${open ? styles.triggerOpen : ""}`}
        onClick={() => !disabled && toggle()}
        disabled={disabled}
        aria-haspopup="menu"
        aria-expanded={open}
      >
        {/* 用 grid 让所有候选 label 占同一格，trigger 宽度跟着最宽的 label 走，
            切换不抖。隐藏的 measure span 只撑宽，可见的 label 居中显示。 */}
        <span className={styles.labelStack}>
          <span className={styles.label}>{current?.label ?? ""}</span>
          {options.map((opt) => (
            <span key={opt.value} className={styles.labelMeasure} aria-hidden>
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
        <div className={styles.menu} data-direction={direction}>
          {options.map((opt) => (
            <button
              key={opt.value}
              type="button"
              className={`${styles.item} ${
                opt.value === value ? styles.itemChecked : ""
              }`}
              onClick={() => {
                onChange(opt.value);
                close();
              }}
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
