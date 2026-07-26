/**
 * AssignDropdown 菜单的定位与限高计算(Issue 1A 修复)。
 *
 * 此前菜单高度不设限:分类一多就长过视口,被屏幕截断的分类永远选不到。
 * 现在高度封顶 + 按所选侧的实际可用空间收紧;超出部分在菜单内部滚动,
 * 常驻侧边滚动条就是"还有更多"的提示(.assignMenu 的 overflow-y + 滚动条样式)。
 *
 * 独立成纯函数模块:不碰 DOM,窗口高度/分类数的各种组合可直接单测。
 */

/** 菜单高度封顶;.assignMenu 不写死 max-height,以这里为唯一真源(经内联样式下发)。 */
export const ASSIGN_MENU_MAX_HEIGHT = 264;

/** 视口边缘留白 */
const MARGIN = 8;
/** trigger 与菜单的垂直间距 */
const GAP = 4;
/** 每行估高:.assignOption 26px + 1px 行间 gap(多算一条 gap,宁可略高提早翻转) */
const ROW_HEIGHT = 27;
/** 菜单自身包装:上下 padding 4×2 + 边框 1×2 */
const ASSIGN_MENU_CHROME = 10;
/** 极矮窗口下的高度兜底:至少能看到约 3 行,配合内滚仍可到达全部选项 */
const MIN_MENU_HEIGHT = 96;

export interface AssignMenuLayout {
  top: number;
  left: number;
  width: number;
  /** 经内联样式下发给菜单的 max-height(px) */
  maxHeight: number;
}

export function assignMenuLayout(args: {
  trigger: { top: number; bottom: number; left: number; width: number };
  viewportHeight: number;
  /** 菜单行数(分类数 + 可能的「取消分类」行) */
  optionCount: number;
}): AssignMenuLayout {
  const { trigger, viewportHeight, optionCount } = args;
  const raw = optionCount * ROW_HEIGHT + ASSIGN_MENU_CHROME;
  const spaceBelow = viewportHeight - trigger.bottom - MARGIN;
  const spaceAbove = trigger.top - MARGIN;
  // 下方装不下(按封顶后的目标高度算)且上方更宽裕时翻到上方
  const wanted = Math.min(raw, ASSIGN_MENU_MAX_HEIGHT);
  const flipUp = spaceBelow < wanted && spaceAbove > spaceBelow;
  const side = flipUp ? spaceAbove : spaceBelow;
  const maxHeight = Math.max(Math.min(ASSIGN_MENU_MAX_HEIGHT, side), MIN_MENU_HEIGHT);
  // 实际渲染高度 = min(内容, maxHeight);翻转时用它锚定 top,菜单底边贴住 trigger
  const height = Math.min(raw, maxHeight);
  return {
    top: flipUp ? Math.max(MARGIN, trigger.top - height - GAP) : trigger.bottom + GAP,
    left: trigger.left,
    width: trigger.width,
    maxHeight,
  };
}
