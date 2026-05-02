import { Section } from "../components/Section";
import { Row } from "../components/Row";
import { DEFAULT_CATEGORIES } from "../../../config/categories";
import styles from "./LabelsTab.module.css";

export default function LabelsTab() {
  return (
    <>
      <Section
        title="分类"
        description="不同进程归属到不同的活动类别，用于统计页按「做了什么」汇总。"
      >
        {DEFAULT_CATEGORIES.map((cat) => (
          <Row
            key={cat.id}
            label={cat.name}
            description={
              cat.apps.length > 0 ? cat.apps.join("、") : "（暂无绑定应用）"
            }
          >
            <span
              className={styles.colorChip}
              style={{ background: cat.color }}
              aria-hidden
            />
            <span className={styles.appCount}>{cat.apps.length}</span>
          </Row>
        ))}
      </Section>

      <Section
        title="未归类"
        description="近 7 天采集到、还没有归类的应用。"
      >
        <Row label="占位" description="待接入数据后展示未归类应用列表。">
          <span style={{ fontSize: 13, color: "var(--text-muted)" }}>—</span>
        </Row>
      </Section>
    </>
  );
}
