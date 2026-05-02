import styles from "./Page.module.css";

export default function Today() {
  return (
    <div className={styles.page}>
      <h1 className={styles.title}>今日总览</h1>
      <p className={styles.subtitle}>今天你在哪些应用上花了多少时间。</p>
      <div className={styles.placeholder}>页面内容待接入</div>
    </div>
  );
}
