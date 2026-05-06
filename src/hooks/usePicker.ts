import { useEffect, useRef, useState, type RefObject } from "react";

export interface PickerState {
  /** 当前是否展开 */
  open: boolean;
  /** trigger + panel 必须挂在这个 ref 的元素内，超出该元素的 mousedown 会触发关闭 */
  wrapRef: RefObject<HTMLDivElement | null>;
  /** 切换展开 */
  toggle: () => void;
  /** 立即关闭 */
  close: () => void;
  /** 立即打开 */
  openMenu: () => void;
}

/**
 * SimplePicker / DevicePicker 共用的展开状态机：
 * - 点外（基于 wrapRef 边界）→ 关
 * - Esc → 关
 * - 监听仅在 open === true 时挂载，避免无谓的全局 listener
 */
export function usePicker(): PickerState {
  const [open, setOpen] = useState(false);
  const wrapRef = useRef<HTMLDivElement | null>(null);

  useEffect(() => {
    if (!open) return;
    const onMouseDown = (e: MouseEvent) => {
      if (wrapRef.current && !wrapRef.current.contains(e.target as Node)) {
        setOpen(false);
      }
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") setOpen(false);
    };
    document.addEventListener("mousedown", onMouseDown);
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("mousedown", onMouseDown);
      document.removeEventListener("keydown", onKey);
    };
  }, [open]);

  return {
    open,
    wrapRef,
    toggle: () => setOpen((v) => !v),
    close: () => setOpen(false),
    openMenu: () => setOpen(true),
  };
}
