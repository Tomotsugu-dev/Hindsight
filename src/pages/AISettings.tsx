import styles from "./Page.module.css";

export default function AISettings() {
  return (
    <div className={styles.page}>
      <h1 className={styles.title}>AI 设置</h1>
      <p className={styles.subtitle}>模型、密钥、提示词与隐私偏好（待规划）。</p>
      <div className={styles.placeholder}>页面内容待接入</div>
    </div>
  );
}
