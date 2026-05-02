import styles from "./Page.module.css";

export default function Sync() {
  return (
    <div className={styles.page}>
      <h1 className={styles.title}>同步</h1>
      <p className={styles.subtitle}>登录账号，把数据同步到云端，跨设备共享。</p>
      <div className={styles.placeholder}>页面内容待接入</div>
    </div>
  );
}
