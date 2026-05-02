import styles from "./Page.module.css";

export default function Week() {
  return (
    <div className={styles.page}>
      <h1 className={styles.title}>周统计</h1>
      <p className={styles.subtitle}>本周（7 天）按日和按应用聚合的使用时长。</p>
      <div className={styles.placeholder}>页面内容待接入</div>
    </div>
  );
}
