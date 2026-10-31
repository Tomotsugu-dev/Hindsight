// What Rust sends the island page. Rust has checked the content, see src-tauri/src/island/.

/** A piece of text in one style. A color too dark for the black island is made lighter here */
export type Run = { text: string; color: string | null; dim: boolean; bold: boolean };

export type Text = { text: Run[]; sub: string | null; size: "small" | "large" };

export type Timer = { start: string; end: string; pausedAt: string | null; color: string | null };

export type Component =
  | { type: "icon"; name: string; color: string | null }
  | ({ type: "text" } & Text)
  | ({ type: "timer" } & Timer)
  | { type: "ring"; value: number; text: string | null; color: string | null }
  | { type: "cells"; colors: (string | null)[]; columns: number; current: number | null }
  | { type: "button"; id: string; icon: string; text: string | null };

export type Content = {
  closed: { left: Component | null; right: Component | null };
  /** `null`: the island does not open */
  open: {
    topRight: Text | null;
    left: Component[];
    right: Component[];
    /** 右边那一行下面，靠右 */
    bottomRight: Text | null;
  } | null;
};

/** In the page: from the top-left corner of the window, y growing downward */
export type PageRect = { x: number; y: number; width: number; height: number };

/** 页面自己画形状时（Windows）要的位置 */
export type Drawn = {
  /** 收起的胶囊 */
  capsule: PageRect;
  /** 展开时左边的内容放在哪：贴着展开的岛的左边 */
  openLeft: PageRect | null;
  /** 往上展开：面板在胶囊那一条上面 */
  up: boolean;
};

/** Where the content of each closed side goes. `topRight`：展开时右上角的字放在哪；`open`：展开
 * 的岛在哪；`drawn`：页面自己画形状时（Windows）要的位置，`null` 时形状由系统的遮罩画（macOS） */
export type Layout = {
  notch: boolean;
  left: PageRect | null;
  right: PageRect | null;
  topRight: PageRect;
  open: PageRect;
  drawn: Drawn | null;
};
