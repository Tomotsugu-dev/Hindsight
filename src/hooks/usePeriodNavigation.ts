import { useCallback, useRef, useState, type RefObject } from "react";

export interface PeriodNavigationState {
  /** 0 = 当前周期；负数 = 历史；不允许 > 0 */
  offset: number;
  /** 滑动动画期间的临时位移（px），由 commit() 在过渡中临时设置 */
  delta: number;
  /** 是否正在过渡中（按钮要 disable） */
  transitioning: boolean;
  /** 能否往未来翻：当前 offset === 0 时不能 */
  canGoForward: boolean;
  /** 滑动 carousel 容器；commit 用它读 clientWidth */
  frameRef: RefObject<HTMLDivElement | null>;
  /** 切到上一/下一周期 */
  commit: (direction: -1 | 1) => void;
  /** 跳回当前周期（offset = 0）；若已在当前则 no-op */
  jumpToCurrent: () => void;
}

/**
 * Today / Week / Month 共用的日期切换状态机。
 *
 * 行为：
 * - 切上一期：先动画到右边界（delta = +width），然后 swipeDuration ms 后无过渡复位（offset--, delta=0）
 * - 切下一期：相反方向
 * - 不允许翻到未来（offset > 0）
 *
 * 视觉细节由调用方提供的 swipeAnimated CSS class 决定，hook 只负责状态机。
 */
export function usePeriodNavigation(opts?: {
  swipeDuration?: number;
}): PeriodNavigationState {
  const swipeDuration = opts?.swipeDuration ?? 420;
  const [offset, setOffset] = useState(0);
  const [delta, setDelta] = useState(0);
  const [transitioning, setTransitioning] = useState(false);
  const frameRef = useRef<HTMLDivElement | null>(null);

  const canGoForward = offset < 0;

  const commit = useCallback(
    (direction: -1 | 1) => {
      if (transitioning) return;
      if (direction === 1 && offset >= 0) return;
      const width = frameRef.current?.clientWidth ?? 0;
      setTransitioning(true);
      setDelta(direction === -1 ? width : -width);
      window.setTimeout(() => {
        setTransitioning(false);
        setOffset((o) => o + direction);
        setDelta(0);
      }, swipeDuration);
    },
    [transitioning, offset, swipeDuration],
  );

  const jumpToCurrent = useCallback(() => {
    if (transitioning || offset === 0) return;
    setOffset(0);
  }, [transitioning, offset]);

  return {
    offset,
    delta,
    transitioning,
    canGoForward,
    frameRef,
    commit,
    jumpToCurrent,
  };
}
