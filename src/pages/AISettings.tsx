import { useState } from "react";
import {
  AlertCircle,
  Check,
  Clock,
  Filter,
  Image as ImageIcon,
  Loader2,
  Server,
  User,
} from "lucide-react";
import { Section } from "./Settings/components/Section";
import { Row } from "./Settings/components/Row";
import { Slider } from "./Settings/components/Slider";
import { SegmentList } from "./Settings/components/SegmentList";
import { CategoryChipMultiSelect } from "./Settings/components/CategoryChipMultiSelect";
import { api, type AiConfig, type AiSegment } from "../api/hindsight";
import { useSettings } from "../state/settings";
import styles from "./AISettings.module.css";

type TestState =
  | { kind: "idle" }
  | { kind: "running" }
  | { kind: "ok"; models: string[] }
  | { kind: "fail"; message: string };

function TestStatus({ state }: { state: TestState }) {
  if (state.kind === "idle") return null;
  if (state.kind === "running") {
    return (
      <span className={styles.testStatus}>
        <Loader2 size={14} strokeWidth={2.2} className={styles.testSpin} />
        测试中…
      </span>
    );
  }
  if (state.kind === "ok") {
    const preview = state.models.slice(0, 3).join("、");
    const more = state.models.length > 3 ? ` …等 ${state.models.length} 个` : "";
    return (
      <span className={`${styles.testStatus} ${styles.testStatusOk}`}>
        <Check size={14} strokeWidth={2.4} />
        已连接{state.models.length > 0 ? `，可用模型：${preview}${more}` : ""}
      </span>
    );
  }
  return (
    <span className={`${styles.testStatus} ${styles.testStatusFail}`}>
      <AlertCircle size={14} strokeWidth={2.2} />
      {state.message}
    </span>
  );
}

export default function AISettings() {
  const { settings, update } = useSettings();
  if (!settings) return null;

  const ai = settings.ai;

  /**
   * 所有 ai 子字段更新都必须走这个 wrapper。
   *
   * 原因：[useSettings.update](../state/settings.tsx) 内部用浅合并
   * `setSettings(prev => ({ ...prev, ...patch }))`。如果直接调
   * `update({ ai: { endpoint: v } })`，settings.ai 整个会被替换成
   * `{ endpoint: v }`，model / segments / 等其他子字段全没了；
   * 后端收到这个 patch 后，#[serde(default)] 会把缺字段填默认值，
   * 把用户已经存好的其他字段彻底擦除。
   *
   * 所以这里 spread 旧 ai 一次，保证发出去的 patch 总是完整 AiConfig。
   */
  const updateAi = (patch: Partial<AiConfig>) => {
    update({ ai: { ...ai, ...patch } });
  };

  const [testState, setTestState] = useState<TestState>({ kind: "idle" });

  const onTest = async () => {
    setTestState({ kind: "running" });
    try {
      const r = await api.testAiEndpoint(
        ai.endpoint,
        ai.apiKey.trim() ? ai.apiKey : undefined,
      );
      if (r.ok) setTestState({ kind: "ok", models: r.models });
      else setTestState({ kind: "fail", message: r.message });
    } catch (e) {
      // 后端理论上把所有异常都包成 ok=false 返回，
      // 这里只兜底前端 invoke 层本身的 reject（极少见）
      setTestState({
        kind: "fail",
        message: e instanceof Error ? e.message : String(e),
      });
    }
  };

  const canTest =
    testState.kind !== "running" && ai.endpoint.trim().length > 0;

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
              value={ai.endpoint}
              onChange={(e) => updateAi({ endpoint: e.target.value })}
              placeholder="http://localhost:11434/v1"
              spellCheck={false}
            />
          </Row>
          <Row label="模型" description="支持 vision 的模型，如 minicpm-v:8b、qwen2-vl 等。">
            <input
              type="text"
              className={styles.textInput}
              value={ai.model}
              onChange={(e) => updateAi({ model: e.target.value })}
              placeholder="minicpm-v:8b"
              spellCheck={false}
            />
          </Row>
          <Row label="API Key" description="可选；Ollama 不需要填。">
            <input
              type="password"
              className={styles.textInput}
              value={ai.apiKey}
              onChange={(e) => updateAi({ apiKey: e.target.value })}
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
                onClick={() => void onTest()}
              >
                {testState.kind === "running" ? "测试中…" : "测试连接"}
              </button>
              <TestStatus state={testState} />
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
              value={ai.userBrief}
              onChange={(e) => updateAi({ userBrief: e.target.value })}
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
            <SegmentList
              segments={ai.segments}
              onChange={(next: AiSegment[]) => updateAi({ segments: next })}
            />
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
              selectedIds={ai.excludedCategories}
              onChange={(next) => updateAi({ excludedCategories: next })}
            />
          </Row>
        </Section>

        <Section
          title="抽帧参数"
          icon={ImageIcon}
          description="一段时间内截图很多，先按相似度去重再选送给模型，省时省 token。"
        >
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
              value={ai.hashThreshold}
              onChange={(v) => updateAi({ hashThreshold: v })}
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
              value={ai.hashWindowMinutes}
              onChange={(v) => updateAi({ hashWindowMinutes: v })}
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
