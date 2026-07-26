/**
 * 自绘滚动条的 thumb 几何(纯函数,便于单测)。
 *
 * 为什么自绘:macOS WKWebView 无视 ::-webkit-scrollbar 自定义样式,原生
 * overlay 条"不滚不显示";要给用户"还有更多内容"的常驻提示(Issue 1A/2A),
 * 双端唯一可靠的路是自己画。隐藏原生条是双端都可靠的,显示才不可靠。
 *
 * 输出为轨道高度的百分比:轨道的像素几何完全交给宿主 CSS,组件与布局解耦。
 */

export interface ThumbGeometry {
  /** thumb 顶边,占轨道高度的百分比 [0, 100) */
  topPct: number;
  /** thumb 高度,占轨道高度的百分比 (0, 100] */
  heightPct: number;
}

/**
 * @param minHeightRatio thumb 最小高度占轨道的比例——超长页面里纯比例
 *   会缩成米粒,拖拽/辨识都难。默认 8%。
 * @returns null = 内容装得下,无需滚动条
 */
export function thumbGeometry(
  scrollTop: number,
  scrollHeight: number,
  clientHeight: number,
  minHeightRatio = 0.08,
): ThumbGeometry | null {
  if (scrollHeight <= clientHeight + 1) return null;
  const heightPct = Math.max((clientHeight / scrollHeight) * 100, minHeightRatio * 100);
  const maxTopPct = 100 - heightPct;
  const scrollable = scrollHeight - clientHeight;
  const topPct = scrollable > 0 ? (scrollTop / scrollable) * maxTopPct : 0;
  // 滚动越界回弹(触控板 rubber-band)时 scrollTop 可短暂为负/超界,钳住
  return { topPct: Math.min(Math.max(topPct, 0), maxTopPct), heightPct };
}
