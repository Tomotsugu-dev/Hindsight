import { useEffect, useRef, type RefObject } from "react";

/** 距离滚动容器上下边缘多少像素时开始自动滚动。 */
export const DRAG_AUTO_SCROLL_EDGE = 56;
/** 每帧最多滚动的像素数。 */
export const DRAG_AUTO_SCROLL_MAX_STEP = 14;

/**
 * 计算当前帧的滚动增量。
 *
 * 中间区域返回 0；进入上下边缘区后按平方根曲线加速，越靠近边缘越快。
 * 光标已经离开容器时仍按最大速度继续滚，这样拖到窗口外也能滚到列表尽头。
 */
export function autoScrollStep(cursorY: number, top: number, bottom: number): number {
  const topDistance = cursorY - top;
  const bottomDistance = bottom - cursorY;

  if (topDistance <= 0) return -DRAG_AUTO_SCROLL_MAX_STEP;
  if (bottomDistance <= 0) return DRAG_AUTO_SCROLL_MAX_STEP;

  if (topDistance < DRAG_AUTO_SCROLL_EDGE) {
    const ratio = 1 - topDistance / DRAG_AUTO_SCROLL_EDGE;
    return -Math.max(1, Math.ceil(DRAG_AUTO_SCROLL_MAX_STEP * Math.sqrt(ratio)));
  }

  if (bottomDistance < DRAG_AUTO_SCROLL_EDGE) {
    const ratio = 1 - bottomDistance / DRAG_AUTO_SCROLL_EDGE;
    return Math.max(1, Math.ceil(DRAG_AUTO_SCROLL_MAX_STEP * Math.sqrt(ratio)));
  }

  return 0;
}

function isVerticalScrollContainer(el: HTMLElement): boolean {
  if (el.scrollHeight <= el.clientHeight + 1) return false;
  const overflowY = getComputedStyle(el).overflowY;
  return overflowY === "auto" || overflowY === "scroll" || overflowY === "overlay";
}

/**
 * 从 start 向上找第一个真正能纵向滚动的祖先。
 *
 * 只看 overflow 不够：`.pairing` 为了横向滚动设了 `overflow-x: auto`，
 * 纵向在规范里也会变成 auto，但它本身高度贴合内容，并不该作为滚动容器。
 */
export function findScrollContainer(start: HTMLElement | null): HTMLElement | null {
  let node = start?.parentElement ?? null;
  while (node) {
    if (isVerticalScrollContainer(node)) return node;
    node = node.parentElement;
  }

  const doc = document.scrollingElement;
  if (doc instanceof HTMLElement && doc.scrollHeight > doc.clientHeight + 1) {
    return doc;
  }
  return null;
}

interface UseDragAutoScrollOptions {
  /** 是否正在拖拽。松手后必须立刻变 false。 */
  active: boolean;
  /** 滚动容器，拖拽开始时用 findScrollContainer 找好。 */
  containerRef: RefObject<HTMLElement | null>;
  /** 当前光标 Y；用 ref 避免每次 mousemove 都重建 rAF 循环。 */
  cursorYRef: RefObject<number>;
  /** 容器发生任何滚动（自动或手动）时触发，用于重新做命中判断。 */
  onScroll?: () => void;
}

export function useDragAutoScroll({
  active,
  containerRef,
  cursorYRef,
  onScroll,
}: UseDragAutoScrollOptions): void {
  const onScrollRef = useRef(onScroll);

  useEffect(() => {
    onScrollRef.current = onScroll;
  }, [onScroll]);

  useEffect(() => {
    if (!active) return;
    const container = containerRef.current;
    if (!container) return;

    let frame = 0;
    let notifyFrame = 0;

    // scrollTop 每帧最多变一次，但 rAF 和 scroll 事件可能同帧都触发。
    // 合并成一次通知，避免同一帧重复重算命中。
    const notify = () => {
      if (notifyFrame !== 0) return;
      notifyFrame = requestAnimationFrame(() => {
        notifyFrame = 0;
        onScrollRef.current?.();
      });
    };

    const tick = () => {
      const rect = container.getBoundingClientRect();
      const step = autoScrollStep(cursorYRef.current, rect.top, rect.bottom);
      if (step !== 0) {
        const before = container.scrollTop;
        container.scrollTop = before + step;
        if (container.scrollTop !== before) notify();
      }
      frame = requestAnimationFrame(tick);
    };

    container.addEventListener("scroll", notify, { passive: true });
    frame = requestAnimationFrame(tick);
    return () => {
      cancelAnimationFrame(frame);
      if (notifyFrame !== 0) cancelAnimationFrame(notifyFrame);
      container.removeEventListener("scroll", notify);
    };
  }, [active, containerRef, cursorYRef]);
}
