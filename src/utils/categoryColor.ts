// 分类色的主题适配：暗色下把高饱和分类色降亮 + 降饱和，避免大面积（柱状图 / 占比环 /
// 进度条 / 大类卡）在深底上"荧光振动"；亮色主题原样返回。
//
// 分类色是后端存的 hex（用户从色板选或自定义），以 inline style / SVG fill 注入，CSS 变量
// 覆盖不到它——所以只能在渲染层按当前主题过一道。用 color-mix 往深灰掺，等价于同时压低
// OKLab 的 L 和 C，且保留色相；color-mix 在本项目已广泛使用，跨平台可靠。
//
// 用法：组件里 `const isDark = useIsDark();` 再 `adjustCategoryColor(color, isDark)`。

/** 掺入的深灰目标——比 card 底（#16161a）略亮一点点，避免柱子和背景糊在一起。 */
const DARK_MIX_TARGET = "#23232b";

/** 保留多少原色（其余掺深灰）。越低越暗越灰。 */
const DARK_KEEP = 72;

/** 暗色下降亮降饱和；亮色 / 空值原样返回。 */
export function adjustCategoryColor(color: string, isDark: boolean): string {
  if (!isDark || !color) return color;
  return `color-mix(in oklab, ${color} ${DARK_KEEP}%, ${DARK_MIX_TARGET})`;
}
