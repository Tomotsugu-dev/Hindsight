import {
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
  type CSSProperties,
} from "react";
import { createPortal } from "react-dom";
import { useTranslation } from "react-i18next";
import {
  Check,
  Cloud,
  Download,
  Eye,
  EyeOff,
  Info,
  Loader2,
  Plus,
  Type,
  X,
  XCircle,
} from "lucide-react";
import { Section } from "../../../components/FormLayout/Section";
import { Row } from "../../../components/FormLayout/Row";
import { Toggle } from "../../../components/FormControls/Toggle";
import { SimplePicker } from "../../../components/SimplePicker/SimplePicker";
import { api, type AiConfig, type ExternalProfile } from "../../../api/hindsight";
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

/**
 * 非文本对话模型的 id 特征:语音(tts/transcribe/whisper/audio/realtime)、
 * 向量(embedding)、图像(dall-e/image/sora)、审核(moderation)、重排(rerank)。
 * 这些发到 /chat/completions 必失败,不进模型建议列表。
 * 边界用 `-`/`_`/`.`/`/` 或首尾,避免误伤名字里含这些词根的对话模型。
 */
const NON_CHAT_MODEL_RE =
  /(^|[-_./])(tts|whisper|transcribe|speech|audio|realtime|embed|embedding|embeddings|moderation|rerank|dall-?e|image|sora|video)([-_./]|$)/i;

function ExternalApiSection({ ai, updateAi }: ExternalApiSectionProps) {
  const { t } = useTranslation();
  const [showKey, setShowKey] = useState(false);
  const [textTest, setTextTest] = useState<ApiTestState>(TEST_IDLE);
  // 「拉取模型」:GET /models 的结果灌进 model 输入框的 datalist(原生可搜)
  const [modelList, setModelList] = useState<string[]>([]);
  const [modelFetch, setModelFetch] = useState<"idle" | "running" | "ok" | "fail">("idle");
  const [modelFetchMsg, setModelFetchMsg] = useState("");
  // 自绘建议下拉(datalist 在 WKWebView 体验极差,弃用):聚焦即开,打字过滤。
  // 菜单 portal 到 body + fixed 定位:设置面板是 overflow 滚动容器,
  // absolute 菜单会被它的底边裁掉半行(同 CategoryFilterDropdown 的路数)
  const [modelMenuOpen, setModelMenuOpen] = useState(false);
  const modelInputRef = useRef<HTMLInputElement>(null);
  const modelMenuRef = useRef<HTMLDivElement>(null);
  const [modelMenuStyle, setModelMenuStyle] = useState<CSSProperties | null>(null);

  // 定位:量输入框矩形,下方空间不够就翻到上方;高度取实际可用空间
  useLayoutEffect(() => {
    if (!modelMenuOpen || !modelInputRef.current) return;
    const r = modelInputRef.current.getBoundingClientRect();
    const gap = 6;
    const margin = 12;
    const below = window.innerHeight - r.bottom - gap - margin;
    const above = r.top - gap - margin;
    const down = below >= 180 || below >= above;
    setModelMenuStyle({
      left: r.left,
      width: r.width,
      maxHeight: Math.max(120, Math.min(280, down ? below : above)),
      // 向上弹用 bottom 锚定:打字过滤让条数变化时,菜单不会离开输入框
      ...(down
        ? { top: r.bottom + gap }
        : { bottom: window.innerHeight - r.top + gap }),
    });
  }, [modelMenuOpen]);

  // 面板滚动/窗口缩放会让 fixed 菜单飘走,直接关掉(菜单自身的滚动除外)
  useEffect(() => {
    if (!modelMenuOpen) return;
    const close = (e: Event) => {
      if (modelMenuRef.current?.contains(e.target as Node)) return;
      setModelMenuOpen(false);
    };
    window.addEventListener("scroll", close, true);
    window.addEventListener("resize", close);
    return () => {
      window.removeEventListener("scroll", close, true);
      window.removeEventListener("resize", close);
    };
  }, [modelMenuOpen]);

  /** 当前激活的四元组是否与某个已存配置一致(chips 高亮用) */
  const profileMatches = (p: ExternalProfile) =>
    p.provider === ai.externalProvider &&
    p.endpoint === ai.endpoint.trim() &&
    p.apiKey === ai.apiKey.trim() &&
    p.model === ai.model.trim();

  const applyProfile = (p: ExternalProfile) => {
    updateAi({
      externalProvider: p.provider || "custom",
      endpoint: p.endpoint,
      apiKey: p.apiKey,
      model: p.model,
    });
    setTextTest(TEST_IDLE);
    setModelList([]);
    setModelFetch("idle");
  };

  const saveCurrentProfile = () => {
    const endpoint = ai.endpoint.trim();
    if (!endpoint || ai.externalProfiles.length >= 10) return;
    if (ai.externalProfiles.some(profileMatches)) return; // 完全相同的不重复存
    let host = endpoint;
    try {
      host = new URL(endpoint).host;
    } catch {
      /* 手填的残缺地址就原样展示 */
    }
    const name = `${ai.externalProvider} · ${ai.model.trim() || host}`;
    updateAi({
      externalProfiles: [
        ...ai.externalProfiles,
        {
          name,
          provider: ai.externalProvider,
          endpoint,
          apiKey: ai.apiKey.trim(),
          model: ai.model.trim(),
        },
      ],
    });
  };

  const deleteProfile = (idx: number) => {
    updateAi({
      externalProfiles: ai.externalProfiles.filter((_, i) => i !== idx),
    });
  };

  /** 拉取云端可用模型:复用 testAiEndpoint(GET /models,上限 500)。
   *  /models 返回的是端点上的**全部**模型(OpenAI 上 130+),语音/向量/
   *  图像/审核类发到 /chat/completions 必失败——这个框只填文本模型,
   *  按 id 特征滤掉,免得用户在一堆 tts/transcribe 里翻找。 */
  const fetchModels = async () => {
    const endpoint = ai.endpoint.trim();
    if (!endpoint) {
      setModelFetch("fail");
      setModelFetchMsg(t("aiSettings.external.missingFields"));
      return;
    }
    setModelFetch("running");
    setModelFetchMsg("");
    try {
      const resp = await api.testAiEndpoint(endpoint, ai.apiKey.trim() || undefined);
      if (!resp.ok) {
        setModelFetch("fail");
        setModelFetchMsg(resp.message);
        return;
      }
      const usable = resp.models.filter((m) => !NON_CHAT_MODEL_RE.test(m));
      setModelList(usable);
      setModelMenuOpen(usable.length > 0);
      setModelFetch("ok");
      setModelFetchMsg(
        t("aiSettings.external.modelsFetched", { count: usable.length }),
      );
    } catch (e) {
      setModelFetch("fail");
      setModelFetchMsg(e instanceof Error ? e.message : String(e));
    }
  };

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
          {ai.externalProfiles.length > 0 && (
            <Row label={t("aiSettings.external.profilesLabel")} block>
              <div className={styles.profileChipRow}>
                {ai.externalProfiles.map((p, i) => (
                  <span key={`${p.endpoint}-${i}`} className={styles.profileChip}>
                    <button
                      type="button"
                      className={styles.profileChipBody}
                      onClick={() => applyProfile(p)}
                      title={p.endpoint}
                    >
                      {p.name || p.endpoint}
                    </button>
                    <button
                      type="button"
                      className={styles.profileChipDel}
                      onClick={() => deleteProfile(i)}
                      aria-label={t("aiSettings.external.deleteProfileAria", {
                        name: p.name || p.endpoint,
                      })}
                    >
                      <X size={10} strokeWidth={2.5} />
                    </button>
                  </span>
                ))}
              </div>
            </Row>
          )}

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
            <div className={styles.modelComboWrap}>
              <div className={styles.externalKeyRow}>
                <input
                  ref={modelInputRef}
                  type="text"
                  className={styles.externalInput}
                  value={ai.model}
                  onChange={(e) => {
                    updateAi({ model: e.target.value });
                    if (modelList.length > 0) setModelMenuOpen(true);
                  }}
                  onFocus={() => {
                    if (modelList.length > 0) setModelMenuOpen(true);
                  }}
                  onBlur={() => setModelMenuOpen(false)}
                  onKeyDown={(e) => {
                    if (e.key === "Escape") setModelMenuOpen(false);
                  }}
                  placeholder={modelHint}
                  spellCheck={false}
                  autoCapitalize="off"
                  autoCorrect="off"
                />
                <button
                  type="button"
                  className={styles.externalEyeBtn}
                  onClick={() => void fetchModels()}
                  disabled={modelFetch === "running"}
                  aria-label={t("aiSettings.external.fetchModels")}
                  title={t("aiSettings.external.fetchModels")}
                >
                  {modelFetch === "running" ? (
                    <Loader2 size={14} strokeWidth={2} className={styles.testSpin} />
                  ) : (
                    <Download size={14} strokeWidth={1.85} />
                  )}
                </button>
              </div>
              {modelMenuOpen &&
                (() => {
                  const q = ai.model.trim().toLowerCase();
                  // 前缀命中排前面:输入 gpt-4o-mini 时精确那条必须在第一行,
                  // 而不是被 gpt-4o-mini-xxx-2025-12-15 挤下去
                  const hits = q
                    ? modelList
                        .filter((m) => m.toLowerCase().includes(q))
                        .sort((a, b) => {
                          const rank = (s: string) =>
                            s.toLowerCase() === q ? 0 : s.toLowerCase().startsWith(q) ? 1 : 2;
                          return rank(a) - rank(b) || a.localeCompare(b);
                        })
                    : modelList;
                  if (hits.length === 0) return null;
                  return createPortal(
                    <div
                      ref={modelMenuRef}
                      className={styles.modelSuggestMenu}
                      role="listbox"
                      // 首帧未测量前先藏,避免定位跳一下
                      style={modelMenuStyle ?? { visibility: "hidden" }}
                    >
                      {hits.slice(0, 100).map((m) => (
                        <button
                          key={m}
                          type="button"
                          role="option"
                          aria-selected={m === ai.model}
                          className={styles.modelSuggestOption}
                          // mousedown + preventDefault:抢在输入框 blur 之前落点
                          onMouseDown={(e) => {
                            e.preventDefault();
                            updateAi({ model: m });
                            setModelMenuOpen(false);
                          }}
                        >
                          {m}
                        </button>
                      ))}
                    </div>,
                    document.body,
                  );
                })()}
            </div>
            {modelFetch === "ok" && (
              <span className={styles.externalTestOk}>{modelFetchMsg}</span>
            )}
            {modelFetch === "fail" && (
              <span className={styles.externalTestFail}>
                <XCircle size={13} strokeWidth={2} />
                {modelFetchMsg}
              </span>
            )}
          </Row>

          <div className={styles.externalActionRow}>
            <button
              type="button"
              className={styles.externalTestBtn}
              onClick={saveCurrentProfile}
              disabled={
                !ai.endpoint.trim() ||
                ai.externalProfiles.length >= 10 ||
                ai.externalProfiles.some(profileMatches)
              }
              title={t("aiSettings.external.saveProfileHint")}
            >
              <Plus size={13} strokeWidth={2.25} />
              {t("aiSettings.external.saveProfile")}
            </button>
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

          <p className={styles.externalPrivacyNote}>
            <Info size={12} strokeWidth={1.85} />
            {t("aiSettings.external.privacyNote")}
          </p>
        </div>
      </div>
    </>
  );
}
