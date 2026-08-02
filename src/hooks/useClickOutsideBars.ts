// Today / Week / Month 三页共用：选中柱子后，点击页面任何**非柱子**区域就清除选中。
//
// 实现思路：当 `active === true` 时挂一个 document-level mousedown 监听，
// 检查事件 target 是不是某个 `[data-bar-button]` 内（柱子按钮自带的标记），
// 如果不是 → 调 onClear。柱子自身的 onClick 还是正常 toggle 行为。
//
// 用 mousedown 而不是 click，是为了在 iOS 那种 touchend 之前就清，避免感觉迟钝。
//
// 例外 `[data-keeps-bar-selection]`：排行榜与应用详情抽屉的内容本身就是从
// 选中态派生的（选了周五，榜单就是周五的），点它们不该反过来把选中清掉。
// 更要命的是时序——mousedown 早于 click，若不放行，点榜单项时选中会先被
// 清空，随后打开的详情抽屉只能拿到整周口径，与用户点的那一天对不上。

import { useEffect } from "react";

/** 放行选择器：柱子本身，以及从选中态派生的联动区域 */
const KEEP_SELECTORS = "[data-bar-button], [data-keeps-bar-selection]";

export function useClickOutsideBars(active: boolean, onClear: () => void) {
  useEffect(() => {
    if (!active) return;
    const handler = (e: MouseEvent) => {
      const target = e.target as HTMLElement | null;
      if (target && target.closest(KEEP_SELECTORS)) return;
      onClear();
    };
    document.addEventListener("mousedown", handler);
    return () => document.removeEventListener("mousedown", handler);
  }, [active, onClear]);
}
