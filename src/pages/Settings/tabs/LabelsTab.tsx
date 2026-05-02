import { Section } from "../components/Section";
import { Row } from "../components/Row";
import styles from "./LabelsTab.module.css";

interface Category {
  id: string;
  name: string;
  color: string;
  apps: string[];
  builtin: boolean;
}

// 内置分类（v0.1 占位数据；后续从持久化存储读）
const DEFAULT_CATEGORIES: Category[] = [
  { id: "code", name: "编程", color: "#8b5cf6", builtin: true,
    apps: ["code.exe", "idea64.exe", "pycharm64.exe", "WebStorm.exe"] },
  { id: "browse", name: "浏览", color: "#3b82f6", builtin: true,
    apps: ["chrome.exe", "firefox.exe", "msedge.exe"] },
  { id: "talk", name: "沟通", color: "#06b6d4", builtin: true,
    apps: ["WeChat.exe", "DingTalk.exe", "Lark.exe", "slack.exe"] },
  { id: "design", name: "设计", color: "#f97316", builtin: true,
    apps: ["Figma.exe", "Photoshop.exe", "Illustrator.exe"] },
  { id: "fun", name: "娱乐", color: "#ec4899", builtin: true,
    apps: ["Spotify.exe", "Steam.exe", "网易云音乐.exe"] },
  { id: "other", name: "其他", color: "#64748b", builtin: true, apps: [] },
];

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
              cat.apps.length > 0
                ? cat.apps.join("、")
                : "（暂无绑定应用）"
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
