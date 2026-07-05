import ReactMarkdown from "react-markdown";
import styles from "./MarkdownText.module.css";

/**
 * LLM 输出的 Markdown 渲染——日报/周报/调试段总结共用。
 * 字号行高继承外层容器（调用方用自己的 bodyText/summaryText 类控制），
 * 这里只负责 Markdown 元素的间距与样式。
 *
 * 刻意**不带** remark-gfm：报告正文里会原样出现窗口标题（含 `~`、`_bilibili_`
 * 之类字符），GFM 的删除线扩展会把 `~…~` 之间整段划掉。纯 CommonMark 没有
 * 删除线语法，加粗/列表/代码照常工作；表格在报告场景不需要。
 */
export function MarkdownText({ text, className }: { text: string; className?: string }) {
  return (
    <div className={`${styles.md}${className ? ` ${className}` : ""}`}>
      <ReactMarkdown>{text}</ReactMarkdown>
    </div>
  );
}
