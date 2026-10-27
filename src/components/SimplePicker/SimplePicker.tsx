import { useLayoutEffect, useState } from "react";
import { Check, ChevronDown, type LucideIcon } from "lucide-react";
import { usePicker } from "../../hooks/usePicker";
import { placeMenu, type MenuPlacement } from "./menuPlacement";
import styles from "./SimplePicker.module.css";

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
  /** 文字左边的图标。筛选用 ListFilter，排序用 ArrowUpDown，全软件统一 */
  icon?: LucideIcon;
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
  icon: Icon,
}: SimplePickerProps<T>) {
  const { open, wrapRef, toggle, close } = usePicker();
  const current = options.find((o) => o.value === value);
  const [placement, setPlacement] = useState<MenuPlacement | null>(null);

  // 打开时按 trigger 在视口里的位置 + 估算菜单高度，决定向哪边展开、最多多高。
  // 避免靠底的 picker（如 EngineTab 的 ctxSize）或选项很多的 picker（如插件版本）
  // 撑出窗口被裁。useLayoutEffect 在 paint 前定位置，避免一帧闪现。
  useLayoutEffect(() => {
    if (!open || !wrapRef.current) return;
    const estMenuHeight = options.length * ITEM_HEIGHT + MENU_PADDING;
    setPlacement(
      placeMenu(
        wrapRef.current.getBoundingClientRect(),
        window.innerHeight,
        estMenuHeight,
        TRIGGER_GAP,
      ),
    );
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
        {Icon ? <Icon size={13} strokeWidth={2} className={styles.icon} aria-hidden /> : null}
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
        <div
          className={styles.menu}
          data-direction={placement?.direction ?? "down"}
          style={{ maxHeight: placement?.maxHeight }}
        >
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
