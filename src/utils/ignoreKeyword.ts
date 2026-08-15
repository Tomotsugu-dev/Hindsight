/**
 * 从窗口标题提炼忽略规则的标题关键词。
 *
 * 终端/工具类窗口的标题常带每帧变化的装饰前缀——Claude Code 的 spinner
 * （⠐/✳/⠂…）会让同一段任务产生几十种只差一个字符的标题；拿整串当关键词，
 * 规则就只能命中其中一种。这里剥掉首尾的装饰，只留文字主体：主体是原标题的
 * 连续子串，后端的子串匹配天然成立。
 *
 * 剥法不对称：前缀连标点一起剥（装饰都在开头），后缀只剥符号/空白/控制符——
 * 「会议纪要(2)」这类结尾的括号是内容，不能吃掉。
 *
 * 全符号标题剥完为空时回退 trim 后原串；空标题返回空串（调用方隐藏按钮）。
 */
export function ignoreKeywordFromTitle(title: string): string {
  const t = title.trim();
  const core = t
    .replace(/^[\p{P}\p{S}\p{Z}\p{C}]+/u, "")
    .replace(/[\p{S}\p{Z}\p{C}]+$/u, "");
  return core || t;
}
