import styles from "./Page.module.css";

export default function Devices() {
  return (
    <div className={styles.page}>
      <h1 className={styles.title}>设备</h1>
      <p className={styles.subtitle}>查看与本账号同步的其他设备及其数据。</p>
      <div className={styles.placeholder}>页面内容待接入</div>
    </div>
  );
}
