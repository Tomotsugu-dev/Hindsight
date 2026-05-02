import styles from "./Page.module.css";

export default function AI() {
  return (
    <div className={styles.page}>
      <h1 className={styles.title}>AI 总结</h1>
      <p className={styles.subtitle}>本地大模型增强分析（待规划）。</p>
      <div className={styles.placeholder}>页面内容待接入</div>
    </div>
  );
}
