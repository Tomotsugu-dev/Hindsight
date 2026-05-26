import { flushSync } from "react-dom";

/**
 * 用 View Transitions API 包一次 state 更新，让浏览器自动给「旧 DOM 快照 → 新 DOM」
 * 之间生成 cross-fade / morph 过渡动画（CSS 里 `::view-transition-*` + `view-transition-name`
 * 共享元素挂在哪里就在哪里 morph）。
 *
 * - `flushSync(fn)` 强制 React 在 snapshot 之前把 commit 跑完，否则 startViewTransition
 *   只能拍到旧 DOM、跑完才看到新 DOM，动画一闪而过
 * - SSR / 老浏览器（Safari ≤17.4 等）没 `startViewTransition`：直接 `fn()` 退化为无过渡
 */
export function withViewTransition(fn: () => void): void {
  if (typeof document === "undefined") {
    fn();
    return;
  }
  const start = (
    document as Document & {
      startViewTransition?: (cb: () => void) => unknown;
    }
  ).startViewTransition;
  if (typeof start !== "function") {
    fn();
    return;
  }
  start.call(document, () => flushSync(fn));
}
