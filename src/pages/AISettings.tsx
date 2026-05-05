import { useState } from "react";
import { Clock, Filter, Image as ImageIcon, Server, User } from "lucide-react";
import { Section } from "./Settings/components/Section";
import { Row } from "./Settings/components/Row";
import { Slider } from "./Settings/components/Slider";
import { SegmentList } from "./Settings/components/SegmentList";
import { CategoryChipMultiSelect } from "./Settings/components/CategoryChipMultiSelect";
import type { AiSegment } from "../api/hindsight";
import styles from "./AISettings.module.css";

const DEFAULT_SEGMENTS: AiSegment[] = [
  { label: "早", startHour: 6, endHour: 9 },
  { label: "上午", startHour: 9, endHour: 12 },
  { label: "下午", startHour: 12, endHour: 18 },
  { label: "晚", startHour: 18, endHour: 24 },
];

export default function AISettings() {
  // Phase 1A 阶段 1：所有字段先用 useState（不持久化），跑 UI 视觉/交互。
  // 阶段 2 接通 settings 后，把这些 state 换成 useSettings + update。
  const [endpoint, setEndpoint] = useState("http://localhost:11434/v1");
  const [model, setModel] = useState("");
  const [apiKey, setApiKey] = useState("");
  const [userBrief, setUserBrief] = useState("");
  const [segments, setSegments] = useState<AiSegment[]>(DEFAULT_SEGMENTS);
  const [excludedCategoryIds, setExcludedCategoryIds] = useState<string[]>([
    "other",
  ]);
  const [maxImagesPerSegment, setMaxImagesPerSegment] = useState(30);
  const [hashThreshold, setHashThreshold] = useState(5);
  const [hashWindowMinutes, setHashWindowMinutes] = useState(5);

  const canTest = endpoint.trim().length > 0;

  return (
    <div className={styles.page}>
      <header className={styles.header}>
        <h1 className={styles.title}>AI 设置</h1>
      </header>

      <div className={styles.content}>
        <Section
          title="后端连接"
          description="OpenAI 兼容接口；本机 Ollama 默认走 11434 端口。"
          icon={Server}
        >
          <Row label="服务地址">
            <input
              type="text"
              className={styles.textInput}
              value={endpoint}
              onChange={(e) => setEndpoint(e.target.value)}
              placeholder="http://localhost:11434/v1"
              spellCheck={false}
            />
          </Row>
          <Row label="模型" description="支持 vision 的模型，如 minicpm-v:8b、qwen2-vl 等。">
            <input
              type="text"
              className={styles.textInput}
              value={model}
              onChange={(e) => setModel(e.target.value)}
              placeholder="minicpm-v:8b"
              spellCheck={false}
            />
          </Row>
          <Row label="API Key" description="可选；Ollama 不需要填。">
            <input
              type="password"
              className={styles.textInput}
              value={apiKey}
              onChange={(e) => setApiKey(e.target.value)}
              placeholder="（可留空）"
              spellCheck={false}
            />
          </Row>
          <Row label="测试连接" block>
            <div className={styles.testRow}>
              <button
                type="button"
                className={styles.testBtn}
                disabled={!canTest}
              >
                测试连接
              </button>
              <span className={styles.testHint}>
                待接入后端（阶段 2）。当前阶段仅校验 UI。
              </span>
            </div>
          </Row>
        </Section>

        <Section
          title="个人简介"
          icon={User}
          info="AI 总结时会带上这段，帮模型更懂你的工作内容与上下文。"
        >
          <Row label="关于你（可选）" block>
            <textarea
              className={styles.textarea}
              value={userBrief}
              onChange={(e) => setUserBrief(e.target.value)}
              placeholder="例：我是做后端开发的，平时主要写 Rust 和 TypeScript；周末会做点游戏。"
              rows={6}
            />
          </Row>
        </Section>

        <Section
          title="时段划分"
          icon={Clock}
          info="AI 按段汇总；段内截图按相似度抽帧再发给模型。"
        >
          <Row label="时段" block>
            <SegmentList segments={segments} onChange={setSegments} />
          </Row>
        </Section>

        <Section title="过滤" icon={Filter}>
          <Row
            label="不分析这些分类"
            labelHint={
              "点击切换：\n" +
              "• 彩色 + 分类图标 = 参与 AI 分析\n" +
              "• 灰色空心 + 闭眼图标 = 已排除"
            }
            block
          >
            <CategoryChipMultiSelect
              selectedIds={excludedCategoryIds}
              onChange={setExcludedCategoryIds}
            />
          </Row>
        </Section>

        <Section
          title="抽帧参数"
          icon={ImageIcon}
          description="一段时间内截图很多，先按相似度去重再选送给模型，省时省 token。"
        >
          <Row label="单段最多图片数">
            <Slider
              value={maxImagesPerSegment}
              onChange={setMaxImagesPerSegment}
              min={1}
              max={100}
              step={1}
              suffix="张"
            />
          </Row>
          <Row
            label="相似度阈值"
            labelHint={
              "dHash 64 位汉明距离\n" +
              "• 越小越严格（同一画面才算重复）\n" +
              "• 5 通常合适\n" +
              "• 0 = 像素级一致才去重"
            }
          >
            <Slider
              value={hashThreshold}
              onChange={setHashThreshold}
              min={0}
              max={32}
              step={1}
            />
          </Row>
          <Row
            label="时间窗"
            labelHint={
              "只在窗口内的截图之间比相似度。\n" +
              "避免把不同时间段的相似画面（如同一应用上午 / 下午）误合并。"
            }
          >
            <Slider
              value={hashWindowMinutes}
              onChange={setHashWindowMinutes}
              min={0}
              max={30}
              step={1}
              suffix="分钟"
            />
          </Row>
        </Section>
      </div>
    </div>
  );
}
