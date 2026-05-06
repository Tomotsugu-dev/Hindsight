import styles from "./PlaceholderTab.module.css";

/** 占位 tab 通用组件——给"周报 / 月报 / 对话"等还没实现的页面用。
 *  给一个明显的虚线卡片提示，避免空白让人以为是 bug。 */
export function PlaceholderTab({
  title,
  hint,
}: {
  title: string;
  hint?: string;
}) {
  return (
    <div className={styles.wrap}>
      <h2 className={styles.title}>{title}</h2>
      <p className={styles.hint}>{hint ?? "功能开发中。"}</p>
    </div>
  );
}
