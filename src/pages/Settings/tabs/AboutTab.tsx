import { Section } from "../components/Section";
import { Row } from "../components/Row";
import styles from "./AboutTab.module.css";

export default function AboutTab() {
  return (
    <>
      <div className={styles.hero}>
        <div className={styles.logo} aria-hidden />
        <div className={styles.heroText}>
          <div className={styles.appName}>Hindsight</div>
          <div className={styles.version}>0.1.0 · Tauri 2 + React</div>
        </div>
      </div>

      <Section title="信息">
        <Row label="作者" description="个人项目，非商用">
          <span className={styles.value}>TomokotoKiyoshi</span>
        </Row>
        <Row label="许可证">
          <span className={styles.value}>MIT</span>
        </Row>
      </Section>

      <Section title="链接">
        <Row label="GitHub 仓库">
          <a href="#" className={styles.link}>查看 →</a>
        </Row>
        <Row label="反馈与建议">
          <a href="#" className={styles.link}>提交 →</a>
        </Row>
      </Section>
    </>
  );
}
