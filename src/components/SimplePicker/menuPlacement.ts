/** 菜单展开方向：下拉默认向下，靠近视口底时自动翻向上。 */
export type MenuDirection = "down" | "up";

export interface MenuPlacement {
  direction: MenuDirection;
  /** 菜单最多多高（px），超出的部分在菜单里滚动 */
  maxHeight: number;
}

/** 菜单离窗口边缘至少留的距离 */
const EDGE_MARGIN = 8;

/** The sizes in SimplePicker.module.css before --density: an item, the menu's padding top and
 * bottom together, and the gap between the trigger and the menu */
const ITEM_HEIGHT = 32;
const MENU_PADDING = 8;
const TRIGGER_GAP = 6;
/** The menu shows at most this many items, and scrolls for the rest */
const MAX_VISIBLE_ITEMS = 8;

export interface MenuSize {
  /** The height of the whole menu */
  height: number;
  /** The highest the menu may be */
  maxHeight: number;
  /** The gap between the trigger and the menu */
  gap: number;
}

/**
 * Returns how tall a menu of `itemCount` items is, before it is drawn.
 *
 * `density` is the --density the menu is drawn with. The CSS multiplies the sizes by it, so
 * this does too; otherwise a compact theme would get a menu placed for the default sizes.
 */
export function menuSize(itemCount: number, density: number): MenuSize {
  const item = ITEM_HEIGHT * density;
  const padding = MENU_PADDING * density;
  return {
    height: itemCount * item + padding,
    maxHeight: MAX_VISIBLE_ITEMS * item + padding,
    gap: TRIGGER_GAP * density,
  };
}

/**
 * 按 trigger 在视口里的位置，决定菜单往哪边展开、最多多高。
 *
 * 下方装得下就向下；装不下时往空间大的一边展开。那一边也装不下（选项很多）时，
 * 菜单高度限制在那一边的空间内，不会撑出窗口被裁掉。
 *
 * @param trigger trigger 在视口里的上下边
 * @param viewportHeight 视口高度
 * @param menuHeight 菜单完整展开的高度（估算）
 * @param gap trigger 到菜单的间距
 */
export function placeMenu(
  trigger: { top: number; bottom: number },
  viewportHeight: number,
  menuHeight: number,
  gap: number,
): MenuPlacement {
  const spaceBelow = viewportHeight - trigger.bottom - gap - EDGE_MARGIN;
  const spaceAbove = trigger.top - gap - EDGE_MARGIN;
  if (spaceBelow >= menuHeight || spaceBelow >= spaceAbove) {
    return { direction: "down", maxHeight: spaceBelow };
  }
  return { direction: "up", maxHeight: spaceAbove };
}
