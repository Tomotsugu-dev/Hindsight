import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Check, Cloud, Eye, EyeOff, Info, Loader2, XCircle } from "lucide-react";
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
  "openrouter",
  "together",
  "groq",
  "custom",
];

type ExternalTestResult =
  | { kind: "idle" }
  | { kind: "running" }
  | { kind: "ok"; count: number }
  | { kind: "fail"; message: string };

export default function ExternalApiTab() {
  const { t } = useTranslation();
  const { ai, updateAi } = useAiSettings();
  if (!ai) return null;

  return (
    <div className={styles.content}>
      <Section
        title={t("aiSettings.external.sectionTitle")}
        icon={Cloud}
        description={t("aiSettings.external.sectionDesc")}
      >
        <ExternalApiSection ai={ai} updateAi={updateAi} />
      </Section>
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
function ExternalApiSection({ ai, updateAi }: ExternalApiSectionProps) {
  const { t } = useTranslation();
  const [showKey, setShowKey] = useState(false);
  const [testResult, setTestResult] = useState<ExternalTestResult>({
    kind: "idle",
  });

  const provider = (PROVIDER_KEYS as string[]).includes(ai.externalProvider)
    ? (ai.externalProvider as ProviderKey)
    : "openai";

  const onProviderChange = (next: ProviderKey) => {
    const preset = EXTERNAL_PROVIDER_PRESETS[next];
    // 切 provider 自动覆盖 baseUrl（让 OpenAI/DeepSeek 切换零摩擦）；
    // model 字段不强制覆盖（避免抹掉用户填好的精确版本号），placeholder 走预设
    updateAi({ externalProvider: next, endpoint: preset.baseUrl });
    setTestResult({ kind: "idle" });
  };

  const onTest = async () => {
    if (!ai.endpoint.trim() || !ai.model.trim()) {
      setTestResult({
        kind: "fail",
        message: t("aiSettings.external.missingFields"),
      });
      return;
    }
    setTestResult({ kind: "running" });
    try {
      const r = await api.testAiEndpoint(
        ai.endpoint.trim(),
        ai.apiKey.trim() || undefined,
      );
      if (r.ok) setTestResult({ kind: "ok", count: r.models.length });
      else setTestResult({ kind: "fail", message: r.message });
    } catch (e) {
      setTestResult({
        kind: "fail",
        message: e instanceof Error ? e.message : String(e),
      });
    }
  };

  const providerOptions = PROVIDER_KEYS.map((k) => ({
    value: k,
    label: t(`aiSettings.external.provider.${k}`),
  }));

  const modelHint = EXTERNAL_PROVIDER_PRESETS[provider].modelHint;

  return (
    <>
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

      {/* 启用 toggle 切换时用 grid-rows 0fr↔1fr trick 做高度过渡 + opacity 淡入：
          DOM 一直 mount，input 内容跟 testResult / showKey 状态都不会被切关后丢失。 */}
      <div
        className={`${styles.externalDetails} ${ai.externalEnabled ? styles.externalDetailsOpen : ""}`}
        aria-hidden={!ai.externalEnabled}
      >
        <div className={styles.externalDetailsInner}>
          <Row label={t("aiSettings.external.providerLabel")}>
            <SimplePicker<ProviderKey>
              value={provider}
              options={providerOptions}
              onChange={onProviderChange}
            />
          </Row>

          <Row label={t("aiSettings.external.baseUrlLabel")} block>
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
              onClick={onTest}
              disabled={testResult.kind === "running"}
            >
              {testResult.kind === "running" ? (
                <>
                  <Loader2
                    size={13}
                    strokeWidth={2}
                    className={styles.testSpin}
                  />
                  {t("aiSettings.external.testRunning")}
                </>
              ) : (
                t("aiSettings.external.testButton")
              )}
            </button>

            {testResult.kind === "ok" ? (
              <span className={styles.externalTestOk}>
                <Check size={13} strokeWidth={2} />
                {testResult.count > 0
                  ? t("aiSettings.external.testOk", {
                      count: testResult.count,
                    })
                  : t("aiSettings.external.testOkNoModels")}
              </span>
            ) : null}

            {testResult.kind === "fail" ? (
              <span className={styles.externalTestFail}>
                <XCircle size={13} strokeWidth={2} />
                {t("aiSettings.external.testFail", {
                  message: testResult.message,
                })}
              </span>
            ) : null}
          </div>

          <p className={styles.externalPrivacyNote}>
            <Info size={12} strokeWidth={1.85} />
            {t("aiSettings.external.privacyNote")}
          </p>
        </div>
      </div>
    </>
  );
}
