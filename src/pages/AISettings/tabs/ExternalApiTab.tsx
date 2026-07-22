import { useState } from "react";
import { useTranslation } from "react-i18next";
import {
  Check,
  Cloud,
  Eye,
  EyeOff,
  Info,
  Loader2,
  ScanEye,
  Type,
  XCircle,
} from "lucide-react";
import { Section } from "../../../components/FormLayout/Section";
import { Row } from "../../../components/FormLayout/Row";
import { Toggle } from "../../../components/FormControls/Toggle";
import { SimplePicker } from "../../../components/SimplePicker/SimplePicker";
import { api, type AiConfig } from "../../../api/hindsight";
import { useAiSettings } from "../shared/useAiSettings";
import styles from "../AISettings.module.css";

/** Provider 预设：选了 provider 就自动填 baseUrl + 把 modelHint 给到输入框 placeholder。
 *  用户仍可手动改 baseUrl / model（非锁定），切回 custom 时清空 baseUrl。 */
type ProviderKey =
  | "openai"
  | "deepseek"
  | "kimi"
  | "kimi-cn"
  | "openrouter"
  | "together"
  | "groq"
  | "custom";

const EXTERNAL_PROVIDER_PRESETS: Record<
  ProviderKey,
  { baseUrl: string; modelHint: string }
> = {
  openai: {
    baseUrl: "https://api.openai.com/v1",
    modelHint: "gpt-4o-mini",
  },
  deepseek: {
    baseUrl: "https://api.deepseek.com/v1",
    modelHint: "deepseek-chat",
  },
  kimi: {
    baseUrl: "https://api.moonshot.ai/v1",
    modelHint: "kimi-k2.6",
  },
  "kimi-cn": {
    baseUrl: "https://api.moonshot.cn/v1",
    modelHint: "kimi-k2.6",
  },
  openrouter: {
    baseUrl: "https://openrouter.ai/api/v1",
    modelHint: "anthropic/claude-3.5-sonnet",
  },
  together: {
    baseUrl: "https://api.together.xyz/v1",
    modelHint: "meta-llama/Llama-3.3-70B-Instruct-Turbo",
  },
  groq: {
    baseUrl: "https://api.groq.com/openai/v1",
    modelHint: "llama-3.3-70b-versatile",
  },
  custom: { baseUrl: "", modelHint: "" },
};

const PROVIDER_KEYS: ProviderKey[] = [
  "openai",
  "deepseek",
  "kimi",
  "kimi-cn",
  "openrouter",
  "together",
  "groq",
  "custom",
];

/** API 测试的分步状态：conn = 网络与认证（GET /models）；chat = 模型真实
 *  可用（POST /chat/completions，max_tokens=1）。
 *  模型 ID 拼错这类问题在 chat 步被服务端 4xx 当场报出来。 */
type StepStatus = "idle" | "running" | "ok" | "fail";

interface ApiTestState {
  running: boolean;
  conn: StepStatus;
  connMsg: string;
  chat: StepStatus;
  chatMsg: string;
}

const TEST_IDLE: ApiTestState = {
  running: false,
  conn: "idle",
  connMsg: "",
  chat: "idle",
  chatMsg: "",
};

export default function ExternalApiTab() {
  const { ai, updateAi } = useAiSettings();
  if (!ai) return null;

  return (
    <div className={styles.content}>
      <ExternalApiSection ai={ai} updateAi={updateAi} />
    </div>
  );
}

interface ExternalApiSectionProps {
  ai: AiConfig;
  updateAi: (patch: Partial<AiConfig>) => void;
}

/**
 * 启用 toggle + provider 选择 + base URL / API key / model ID 三个输入框
 * + 测试连接按钮 + 隐私 hint。
 *
 * 测试连接复用 api.testAiEndpoint（GET /v1/models）；toggle 关闭时只渲染
 * Toggle 一行，省得把空 / 用户填了一半的字段也露出来。
 */
/** 测试步骤行：转圈/✓/✗ 图标 + 标签；失败时把服务端错误拼在标签后。 */
function TestStepRow({
  status,
  label,
  msg,
}: {
  status: StepStatus;
  label: string;
  msg: string;
}) {
  if (status === "idle") return null;
  const cls =
    status === "ok"
      ? styles.externalTestOk
      : status === "fail"
        ? styles.externalTestFail
        : styles.externalTestPending;
  return (
    <span className={cls}>
      {status === "running" ? (
        <Loader2 size={13} strokeWidth={2} className={styles.testSpin} />
      ) : status === "ok" ? (
        <Check size={13} strokeWidth={2} />
      ) : (
        <XCircle size={13} strokeWidth={2} />
      )}
      {msg ? `${label}：${msg}` : label}
    </span>
  );
}

function ExternalApiSection({ ai, updateAi }: ExternalApiSectionProps) {
  const { t } = useTranslation();
  const [showKey, setShowKey] = useState(false);
  const [textTest, setTextTest] = useState<ApiTestState>(TEST_IDLE);

  const provider = (PROVIDER_KEYS as string[]).includes(ai.externalProvider)
    ? (ai.externalProvider as ProviderKey)
    : "openai";

  const onProviderChange = (next: ProviderKey) => {
    const preset = EXTERNAL_PROVIDER_PRESETS[next];
    // 切 provider 自动覆盖 baseUrl（让 OpenAI/DeepSeek 切换零摩擦）；
    // model 字段不强制覆盖（避免抹掉用户填好的精确版本号），placeholder 走预设
    updateAi({ externalProvider: next, endpoint: preset.baseUrl });
    setTextTest(TEST_IDLE);
  };

  /** 测试文本 API：先测连通（GET /models）再真发一次 chat 验证模型 ID。 */
  const runApiTest = async () => {
    const endpoint = ai.endpoint.trim();
    const key = ai.apiKey.trim();
    const model = ai.model.trim();
    const set = setTextTest;

    if (!endpoint || !model) {
      set({
        running: false,
        conn: "fail",
        connMsg: t("aiSettings.external.missingFields"),
        chat: "idle",
        chatMsg: "",
      });
      return;
    }

    set({
      running: true,
      conn: "running",
      connMsg: "",
      chat: "idle",
      chatMsg: "",
    });
    let conn;
    try {
      conn = await api.testAiEndpoint(endpoint, key || undefined);
    } catch (e) {
      set({
        running: false,
        conn: "fail",
        connMsg: e instanceof Error ? e.message : String(e),
        chat: "idle",
        chatMsg: "",
      });
      return;
    }
    if (!conn.ok) {
      set({
        running: false,
        conn: "fail",
        connMsg: conn.message,
        chat: "idle",
        chatMsg: "",
      });
      return;
    }

    set({ running: true, conn: "ok", connMsg: "", chat: "running", chatMsg: "" });
    let chat;
    try {
      chat = await api.testAiChat(endpoint, key || undefined, model, false);
    } catch (e) {
      set({
        running: false,
        conn: "ok",
        connMsg: "",
        chat: "fail",
        chatMsg: e instanceof Error ? e.message : String(e),
      });
      return;
    }
    set({
      running: false,
      conn: "ok",
      connMsg: "",
      chat: chat.ok ? "ok" : "fail",
      chatMsg: chat.ok ? "" : chat.message,
    });
  };

  const providerOptions = PROVIDER_KEYS.map((k) => ({
    value: k,
    label: t(`aiSettings.external.provider.${k}`),
  }));

  const modelHint = EXTERNAL_PROVIDER_PRESETS[provider].modelHint;

  return (
    <>
      <Section
        title={t("aiSettings.external.sectionTitle")}
        icon={Cloud}
        description={t("aiSettings.external.sectionDesc")}
      >
        <Row
          label={t("aiSettings.external.enableLabel")}
          description={t("aiSettings.external.enableHint")}
        >
          <Toggle
            checked={ai.externalEnabled}
            onChange={(next) => updateAi({ externalEnabled: next })}
            ariaLabel={t("aiSettings.external.enableLabel")}
          />
        </Row>
      </Section>

      {/* 启用 toggle 切换时用 grid-rows 0fr↔1fr trick 做高度过渡 + opacity 淡入：
          DOM 一直 mount，input 内容跟 textTest / showKey 状态都不会被切关后丢失。 */}
      <div
        className={`${styles.externalDetails} ${ai.externalEnabled ? styles.externalDetailsOpen : ""}`}
        aria-hidden={!ai.externalEnabled}
      >
        <div className={styles.externalDetailsInner}>
          <Section
            title={t("aiSettings.external.groupTextTitle")}
            icon={Type}
            description={t("aiSettings.external.groupTextHint")}
          >
          <Row label={t("aiSettings.external.providerLabel")}>
            <SimplePicker<ProviderKey>
              value={provider}
              options={providerOptions}
              onChange={onProviderChange}
            />
          </Row>

          <Row
            label={t("aiSettings.external.baseUrlLabel")}
            description={
              provider === "kimi" || provider === "kimi-cn"
                ? t("aiSettings.external.kimiBaseUrlNote")
                : undefined
            }
            block
          >
            <input
              type="text"
              className={styles.externalInput}
              value={ai.endpoint}
              onChange={(e) => updateAi({ endpoint: e.target.value })}
              placeholder={t("aiSettings.external.baseUrlPlaceholder")}
              spellCheck={false}
              autoCapitalize="off"
              autoCorrect="off"
            />
          </Row>

          <Row label={t("aiSettings.external.apiKeyLabel")} block>
            <div className={styles.externalKeyRow}>
              <input
                type={showKey ? "text" : "password"}
                className={styles.externalInput}
                value={ai.apiKey}
                onChange={(e) => updateAi({ apiKey: e.target.value })}
                placeholder={t("aiSettings.external.apiKeyPlaceholder")}
                spellCheck={false}
                autoCapitalize="off"
                autoCorrect="off"
              />
              <button
                type="button"
                className={styles.externalEyeBtn}
                onClick={() => setShowKey((v) => !v)}
                aria-label={
                  showKey
                    ? t("aiSettings.external.apiKeyHide")
                    : t("aiSettings.external.apiKeyShow")
                }
                title={
                  showKey
                    ? t("aiSettings.external.apiKeyHide")
                    : t("aiSettings.external.apiKeyShow")
                }
              >
                {showKey ? (
                  <EyeOff size={14} strokeWidth={1.85} />
                ) : (
                  <Eye size={14} strokeWidth={1.85} />
                )}
              </button>
            </div>
          </Row>

          <Row label={t("aiSettings.external.modelLabel")} block>
            <input
              type="text"
              className={styles.externalInput}
              value={ai.model}
              onChange={(e) => updateAi({ model: e.target.value })}
              placeholder={modelHint}
              spellCheck={false}
              autoCapitalize="off"
              autoCorrect="off"
            />
          </Row>

          <div className={styles.externalActionRow}>
            <button
              type="button"
              className={styles.externalTestBtn}
              onClick={() => void runApiTest()}
              disabled={textTest.running}
            >
              {textTest.running ? (
                <>
                  <Loader2
                    size={13}
                    strokeWidth={2}
                    className={styles.testSpin}
                  />
                  {t("aiSettings.external.testRunning")}
                </>
              ) : (
                t("aiSettings.external.testTextButton")
              )}
            </button>
          </div>
          {textTest.conn !== "idle" ? (
            <div className={styles.externalTestSteps}>
              <TestStepRow
                status={textTest.conn}
                label={t("aiSettings.external.testStepConn")}
                msg={textTest.connMsg}
              />
              <TestStepRow
                status={textTest.chat}
                label={t("aiSettings.external.testStepChat", {
                  model: ai.model.trim() || "?",
                })}
                msg={textTest.chatMsg}
              />
            </div>
          ) : null}

          </Section>

          <VisionSection ai={ai} updateAi={updateAi} />

          <p className={styles.externalPrivacyNote}>
            <Info size={12} strokeWidth={1.85} />
            {t("aiSettings.external.privacyNote")}
          </p>
        </div>
      </div>
    </>
  );
}

/** 视觉模型(截图洞察用):默认复用文本端点,只填模型名;
 *  取消复用时展开独立 Endpoint / Key。测试发本地合成色块图,不上传真实截图。 */
function VisionSection({ ai, updateAi }: ExternalApiSectionProps) {
  const { t } = useTranslation();
  const [showKey, setShowKey] = useState(false);
  const [test, setTest] = useState<{
    running: boolean;
    status: "idle" | "ok" | "fail";
    msg: string;
  }>({ running: false, status: "idle", msg: "" });

  const endpoint = ai.visionReuseText ? ai.endpoint : ai.visionEndpoint;
  const apiKey = ai.visionReuseText ? ai.apiKey : ai.visionApiKey;

  const runTest = async () => {
    setTest({ running: true, status: "idle", msg: "" });
    try {
      const reply = await api.testAiVision(
        endpoint.trim(),
        apiKey.trim() || undefined,
        ai.visionModel.trim(),
      );
      setTest({ running: false, status: "ok", msg: reply });
    } catch (e) {
      setTest({ running: false, status: "fail", msg: String(e) });
    }
  };

  return (
    <Section
      title={t("aiSettings.external.groupVisionTitle")}
      icon={ScanEye}
      description={t("aiSettings.external.groupVisionHint")}
    >
      <Row
        label={t("aiSettings.external.visionReuseLabel")}
        description={t("aiSettings.external.visionReuseHint")}
      >
        <Toggle
          checked={ai.visionReuseText}
          onChange={(next) => updateAi({ visionReuseText: next })}
          ariaLabel={t("aiSettings.external.visionReuseLabel")}
        />
      </Row>
      {!ai.visionReuseText ? (
        <>
          <Row label={t("aiSettings.external.baseUrlLabel")} block>
            <input
              type="text"
              className={styles.externalInput}
              value={ai.visionEndpoint}
              onChange={(e) => updateAi({ visionEndpoint: e.target.value })}
              placeholder={t("aiSettings.external.baseUrlPlaceholder")}
              spellCheck={false}
              autoCapitalize="off"
              autoCorrect="off"
            />
          </Row>
          <Row label={t("aiSettings.external.apiKeyLabel")} block>
            <div className={styles.externalKeyRow}>
              <input
                type={showKey ? "text" : "password"}
                className={styles.externalInput}
                value={ai.visionApiKey}
                onChange={(e) => updateAi({ visionApiKey: e.target.value })}
                placeholder={t("aiSettings.external.apiKeyPlaceholder")}
                spellCheck={false}
                autoCapitalize="off"
                autoCorrect="off"
              />
              <button
                type="button"
                className={styles.externalEyeBtn}
                onClick={() => setShowKey((v) => !v)}
                aria-label={t("aiSettings.external.apiKeyShow")}
              >
                {showKey ? (
                  <EyeOff size={14} strokeWidth={1.85} />
                ) : (
                  <Eye size={14} strokeWidth={1.85} />
                )}
              </button>
            </div>
          </Row>
        </>
      ) : null}
      <Row label={t("aiSettings.external.visionModelLabel")} block>
        <input
          type="text"
          className={styles.externalInput}
          value={ai.visionModel}
          onChange={(e) => updateAi({ visionModel: e.target.value })}
          placeholder="Qwen/Qwen3-VL-8B-Instruct"
          spellCheck={false}
          autoCapitalize="off"
          autoCorrect="off"
        />
      </Row>
      <div className={styles.externalActionRow}>
        <button
          type="button"
          className={styles.externalTestBtn}
          onClick={() => void runTest()}
          disabled={
            test.running || !endpoint.trim() || !ai.visionModel.trim()
          }
        >
          {test.running ? (
            <>
              <Loader2 size={13} strokeWidth={2} className={styles.testSpin} />
              {t("aiSettings.external.testRunning")}
            </>
          ) : (
            t("aiSettings.external.testVisionButton")
          )}
        </button>
      </div>
      {test.status !== "idle" ? (
        <div className={styles.externalTestSteps}>
          <TestStepRow
            status={test.status}
            label={t("aiSettings.external.testStepVision")}
            msg={test.msg}
          />
        </div>
      ) : null}
      <p className={styles.externalPrivacyNote}>
        <Info size={12} strokeWidth={1.85} />
        {t("aiSettings.external.visionTestNote")}
      </p>
    </Section>
  );
}
