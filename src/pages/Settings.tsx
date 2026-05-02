import styles from "./Page.module.css";

export default function Settings() {
  return (
    <div className={styles.page}>
      <h1 className={styles.title}>设置</h1>
      <p className={styles.subtitle}>采集间隔、保存路径、开机自启等。</p>
      <div className={styles.placeholder}>页面内容待接入</div>
    </div>
  );
}
