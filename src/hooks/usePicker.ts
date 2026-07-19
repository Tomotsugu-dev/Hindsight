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
    // 捕获阶段监听:弹窗(如导出对话框)会在自己面板上 stopPropagation 挡背板
    // 关闭,冒泡监听在弹窗内部收不到 mousedown——"弹窗内、picker 外"的点击
    // 就关不掉菜单。捕获阶段自上而下先于一切 stopPropagation 执行,不受影响。
    document.addEventListener("mousedown", onMouseDown, true);
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("mousedown", onMouseDown, true);
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
