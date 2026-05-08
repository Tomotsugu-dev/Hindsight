// Today / Week / Month 三页共用：选中柱子后，点击页面任何**非柱子**区域就清除选中。
//
// 实现思路：当 `active === true` 时挂一个 document-level mousedown 监听，
// 检查事件 target 是不是某个 `[data-bar-button]` 内（柱子按钮自带的标记），
// 如果不是 → 调 onClear。柱子自身的 onClick 还是正常 toggle 行为。
//
// 用 mousedown 而不是 click，是为了在 iOS 那种 touchend 之前就清，避免感觉迟钝。

import { useEffect } from "react";

export function useClickOutsideBars(active: boolean, onClear: () => void) {
  useEffect(() => {
    if (!active) return;
    const handler = (e: MouseEvent) => {
      const target = e.target as HTMLElement | null;
      if (target && target.closest("[data-bar-button]")) return;
      onClear();
    };
    document.addEventListener("mousedown", handler);
    return () => document.removeEventListener("mousedown", handler);
  }, [active, onClear]);
}
