/**
 * 按界面语言挑插件简介：当前语言 → 英文 → 语言代码排序后的第一个 → 空串。
 * 语言代码要完全一致，繁体（zh-TW）不拿简体（zh-CN）顶替。
 */
export function pickDescription(description: Record<string, string>, locale: string): string {
  const first = Object.keys(description).sort()[0];
  return description[locale] ?? description.en ?? (first === undefined ? "" : description[first]);
}
