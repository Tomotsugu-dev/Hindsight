import { useEffect, useId, useLayoutEffect, useRef, useState, type CSSProperties } from "react";
import { createPortal } from "react-dom";
import { useTranslation } from "react-i18next";
import {
  Activity,
  Check,
  Cloud,
  Download,
  Eye,
  EyeOff,
  Info,
  Loader2,
  Plus,
  XCircle,
} from "lucide-react";
import { Button } from "../../../../components/Button/Button";
import { IconButton } from "../../../../components/Button/IconButton";
import { TextInput } from "../../../../components/TextInput/TextInput";
import { Section } from "../../../../components/FormLayout/Section";
import { SimplePicker } from "../../../../components/SimplePicker/SimplePicker";
import { api, type AiConfig, type CloudConnection } from "../../../../api/hindsight";
import { useAiSettings } from "../../../AISettings/shared/useAiSettings";
import { profileLabel } from "../../../../utils/profileLabel";
import { isCloudConnectionReady } from "../../../Chat/chatRouting";
import styles from "../../../AISettings/AISettings.module.css";
import s from "./aiModels.module.css";

/** At most this many connections can be saved. */
const MAX_CONNECTIONS = 10;

/** What the cloud form shows: nothing, a new connection, or a saved one. */
export type CloudEditor = { mode: "closed" } | { mode: "new" } | { mode: "edit"; id: string };

type ProviderKey =
  | "openai"
  | "deepseek"
  | "kimi"
  | "kimi-cn"
  | "openrouter"
  | "together"
  | "groq"
  | "custom";

const EXTERNAL_PROVIDER_PRESETS: Record<ProviderKey, { baseUrl: string; modelHint: string }> = {
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

const NON_CHAT_MODEL_RE =
  /(^|[-_./])(tts|whisper|transcribe|speech|audio|realtime|embed|embedding|embeddings|moderation|rerank|dall-?e|image|sora|video)([-_./]|$)/i;

function newConnectionId(existing: CloudConnection[]): string {
  const used = new Set(existing.map((c) => c.id));
  for (let i = 1; i <= 1000; i++) {
    const id = `c${i}`;
    if (!used.has(id)) return id;
  }
  return `c_${Date.now().toString(36)}`;
}

interface DraftFields {
  dialect: ProviderKey;
  endpoint: string;
  apiKey: string;
  model: string;
}

const EMPTY_DRAFT: DraftFields = {
  dialect: "openai",
  endpoint: EXTERNAL_PROVIDER_PRESETS.openai.baseUrl,
  apiKey: "",
  model: "",
};

function toProviderKey(raw: string): ProviderKey {
  return (PROVIDER_KEYS as string[]).includes(raw) ? (raw as ProviderKey) : "custom";
}

function TestStepRow({ status, label, msg }: { status: StepStatus; label: string; msg: string }) {
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

interface CloudApiSectionProps {
  editor: CloudEditor;
  onEditorChange: (next: CloudEditor) => void;
}

/**
 * Add a cloud API connection, or change a saved one.
 *
 * Saved connections show as tags. Clicking a tag opens its form below, and clicking it again
 * closes the form. Edit in My models opens the same form. Changes to a saved connection save as
 * the user types.
 */
export function CloudApiSection({ editor, onEditorChange }: CloudApiSectionProps) {
  const { t } = useTranslation();
  const { ai, updateAi } = useAiSettings();
  if (!ai) return null;

  const editing = editor.mode === "edit" ? ai.cloud.find((c) => c.id === editor.id) : undefined;

  return (
    <Section
      title={t("settings.aiModels.cloud.title")}
      description={t("aiSettings.external.sectionDesc")}
    >
      <CloudTags connections={ai.cloud} editor={editor} onEditorChange={onEditorChange} />
      {editor.mode === "new" || editing ? (
        <CloudForm
          // 换一个配置就重来一遍：草稿、测试结果、拿到的模型列表都清掉
          key={editing?.id ?? "new"}
          ai={ai}
          updateAi={updateAi}
          editing={editing ?? null}
          onClose={() => onEditorChange({ mode: "closed" })}
        />
      ) : null}
    </Section>
  );
}

/** The saved connections as tags, then a tag that adds one. The tag of the open form is pressed. */
function CloudTags({
  connections,
  editor,
  onEditorChange,
}: {
  connections: CloudConnection[];
  editor: CloudEditor;
  onEditorChange: (next: CloudEditor) => void;
}) {
  const { t } = useTranslation();
  const full = connections.length >= MAX_CONNECTIONS;
  const adding = editor.mode === "new";

  return (
    <div className={s.cloudTags}>
      {connections.length === 0 ? (
        <span className={s.cloudEmpty}>
          {t("settings.aiModels.cloud.empty", { max: MAX_CONNECTIONS })}
        </span>
      ) : null}
      {connections.map((conn) => {
        const open = editor.mode === "edit" && editor.id === conn.id;
        return (
          <button
            key={conn.id}
            type="button"
            className={s.tag}
            aria-pressed={open}
            title={conn.endpoint}
            onClick={() =>
              onEditorChange(open ? { mode: "closed" } : { mode: "edit", id: conn.id })
            }
          >
            <Cloud aria-hidden />
            <span className={s.tagText}>{profileLabel(conn)}</span>
            {isCloudConnectionReady(conn) ? null : (
              <span className={`${s.cap} ${s.capWarn}`}>
                {t("settings.aiModels.mine.cloudIncomplete")}
              </span>
            )}
          </button>
        );
      })}
      <button
        type="button"
        className={`${s.tag} ${s.tagAdd}`}
        aria-pressed={adding}
        disabled={full && !adding}
        title={full ? t("settings.aiModels.cloud.full", { max: MAX_CONNECTIONS }) : undefined}
        onClick={() => onEditorChange(adding ? { mode: "closed" } : { mode: "new" })}
      >
        <Plus aria-hidden />
        <span className={s.tagText}>{t("settings.aiModels.cloud.add")}</span>
      </button>
      {connections.length > 0 ? (
        <span className={s.cloudCount}>
          {connections.length} / {MAX_CONNECTIONS}
        </span>
      ) : null}
    </div>
  );
}

interface CloudFormProps {
  ai: AiConfig;
  updateAi: (patch: Partial<AiConfig>) => void;
  /** The saved connection being changed; null for a new one. */
  editing: CloudConnection | null;
  onClose: () => void;
}

function CloudForm({ ai, updateAi, editing, onClose }: CloudFormProps) {
  const { t } = useTranslation();
  const fieldId = useId();
  const [draft, setDraft] = useState<DraftFields>(EMPTY_DRAFT);
  const [showKey, setShowKey] = useState(false);
  const [textTest, setTextTest] = useState<ApiTestState>(TEST_IDLE);
  const [modelList, setModelList] = useState<string[]>([]);
  const [modelFetch, setModelFetch] = useState<"idle" | "running" | "ok" | "fail">("idle");
  const [modelFetchMsg, setModelFetchMsg] = useState("");
  const [modelMenuOpen, setModelMenuOpen] = useState(false);
  const modelInputRef = useRef<HTMLInputElement>(null);
  const modelMenuRef = useRef<HTMLDivElement>(null);
  const [modelMenuStyle, setModelMenuStyle] = useState<CSSProperties | null>(null);

  const selectedConn = editing;

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
      ...(down ? { top: r.bottom + gap } : { bottom: window.innerHeight - r.top + gap }),
    });
  }, [modelMenuOpen]);

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

  const activeFields: DraftFields = selectedConn
    ? {
        dialect: toProviderKey(selectedConn.dialect),
        endpoint: selectedConn.endpoint,
        apiKey: selectedConn.apiKey,
        model: selectedConn.model,
      }
    : draft;

  const updateFields = (patch: Partial<DraftFields>) => {
    if (selectedConn) {
      const merged: CloudConnection = {
        ...selectedConn,
        ...patch,
      };
      merged.name = profileLabel(merged);
      updateAi({
        cloud: ai.cloud.map((c) => (c.id === selectedConn.id ? merged : c)),
      });
    } else {
      setDraft((prev) => ({ ...prev, ...patch }));
    }
  };

  const saveAsConnection = () => {
    const endpoint = activeFields.endpoint.trim();
    if (!endpoint || ai.cloud.length >= MAX_CONNECTIONS) return;
    const id = newConnectionId(ai.cloud);
    const conn: CloudConnection = {
      id,
      name: profileLabel({
        dialect: activeFields.dialect,
        endpoint,
        model: activeFields.model.trim(),
      }),
      dialect: activeFields.dialect,
      endpoint,
      model: activeFields.model.trim(),
      apiKey: activeFields.apiKey.trim(),
      contextLength: null,
    };
    updateAi({ cloud: [...ai.cloud, conn] });
    // 存好的配置在「我的模型」里列出来，表单收起
    onClose();
  };

  const sameAsAnySaved = ai.cloud.some(
    (c) =>
      c.dialect === activeFields.dialect &&
      c.endpoint.trim() === activeFields.endpoint.trim() &&
      c.apiKey.trim() === activeFields.apiKey.trim() &&
      c.model.trim() === activeFields.model.trim(),
  );

  const fetchModels = async () => {
    const endpoint = activeFields.endpoint.trim();
    if (!endpoint) {
      setModelFetch("fail");
      setModelFetchMsg(t("aiSettings.external.missingFields"));
      return;
    }
    setModelFetch("running");
    setModelFetchMsg("");
    try {
      const resp = await api.testAiEndpoint(endpoint, activeFields.apiKey.trim() || undefined);
      if (!resp.ok) {
        setModelFetch("fail");
        setModelFetchMsg(resp.message);
        return;
      }
      const usable = resp.models.filter((m) => !NON_CHAT_MODEL_RE.test(m));
      setModelList(usable);
      setModelMenuOpen(usable.length > 0);
      setModelFetch("ok");
      setModelFetchMsg(t("aiSettings.external.modelsFetched", { count: usable.length }));
    } catch (e) {
      setModelFetch("fail");
      setModelFetchMsg(e instanceof Error ? e.message : String(e));
    }
  };

  const onProviderChange = (next: ProviderKey) => {
    const preset = EXTERNAL_PROVIDER_PRESETS[next];
    updateFields({ dialect: next, endpoint: preset.baseUrl });
    setTextTest(TEST_IDLE);
  };

  const runApiTest = async () => {
    const endpoint = activeFields.endpoint.trim();
    const key = activeFields.apiKey.trim();
    const model = activeFields.model.trim();

    if (!endpoint || !model) {
      setTextTest({
        running: false,
        conn: "fail",
        connMsg: t("aiSettings.external.missingFields"),
        chat: "idle",
        chatMsg: "",
      });
      return;
    }

    setTextTest({
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
      setTextTest({
        running: false,
        conn: "fail",
        connMsg: e instanceof Error ? e.message : String(e),
        chat: "idle",
        chatMsg: "",
      });
      return;
    }
    if (!conn.ok) {
      setTextTest({
        running: false,
        conn: "fail",
        connMsg: conn.message,
        chat: "idle",
        chatMsg: "",
      });
      return;
    }

    setTextTest({ running: true, conn: "ok", connMsg: "", chat: "running", chatMsg: "" });
    let chat;
    try {
      chat = await api.testAiChat(endpoint, key || undefined, model, false);
    } catch (e) {
      setTextTest({
        running: false,
        conn: "ok",
        connMsg: "",
        chat: "fail",
        chatMsg: e instanceof Error ? e.message : String(e),
      });
      return;
    }
    setTextTest({
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
  const modelHint = EXTERNAL_PROVIDER_PRESETS[activeFields.dialect].modelHint;

  return (
    <div className={s.panel}>
      <h3 className={s.panelTitle}>
        {editing
          ? t("settings.aiModels.cloud.editing", { name: profileLabel(editing) })
          : t("settings.aiModels.cloud.add")}
      </h3>
      <p className={s.note}>
        <Info size={13} strokeWidth={1.85} />
        {t("aiSettings.external.privacyNote")}
      </p>

      <div className={s.field}>
        <span className={s.label}>{t("aiSettings.external.providerLabel")}</span>
        <div>
          <SimplePicker<ProviderKey>
            value={activeFields.dialect}
            options={providerOptions}
            onChange={onProviderChange}
          />
        </div>
      </div>

      <div className={s.field}>
        <label className={s.label} htmlFor={`${fieldId}-url`}>
          {t("aiSettings.external.baseUrlLabel")}
        </label>
        <TextInput
          id={`${fieldId}-url`}
          className={styles.externalField}
          value={activeFields.endpoint}
          onChange={(e) => updateFields({ endpoint: e.target.value })}
          placeholder={t("aiSettings.external.baseUrlPlaceholder")}
          spellCheck={false}
          autoCapitalize="off"
          autoCorrect="off"
        />
        {activeFields.dialect === "kimi" || activeFields.dialect === "kimi-cn" ? (
          <span className={s.help}>{t("aiSettings.external.kimiBaseUrlNote")}</span>
        ) : null}
      </div>

      <div className={s.field}>
        <label className={s.label} htmlFor={`${fieldId}-key`}>
          {t("aiSettings.external.apiKeyLabel")}
        </label>
        <div className={styles.externalKeyRow}>
          <TextInput
            id={`${fieldId}-key`}
            type={showKey ? "text" : "password"}
            className={styles.externalField}
            value={activeFields.apiKey}
            onChange={(e) => updateFields({ apiKey: e.target.value })}
            placeholder={t("aiSettings.external.apiKeyPlaceholder")}
            spellCheck={false}
            autoCapitalize="off"
            autoCorrect="off"
          />
          <IconButton
            variant="soft"
            icon={showKey ? EyeOff : Eye}
            onClick={() => setShowKey((v) => !v)}
            label={
              showKey ? t("aiSettings.external.apiKeyHide") : t("aiSettings.external.apiKeyShow")
            }
          />
        </div>
      </div>

      <div className={s.field}>
        <label className={s.label} htmlFor={`${fieldId}-model`}>
          {t("aiSettings.external.modelLabel")}
        </label>
        <div className={styles.modelComboWrap}>
          <div className={styles.externalKeyRow}>
            <input
              ref={modelInputRef}
              id={`${fieldId}-model`}
              type="text"
              className={styles.externalInput}
              value={activeFields.model}
              onChange={(e) => {
                updateFields({ model: e.target.value });
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
            <IconButton
              variant="soft"
              icon={Download}
              onClick={() => void fetchModels()}
              loading={modelFetch === "running"}
              label={t("aiSettings.external.fetchModels")}
            />
          </div>
          {modelMenuOpen &&
            (() => {
              const q = activeFields.model.trim().toLowerCase();
              const hits = q
                ? modelList
                    .filter((m) => m.toLowerCase().includes(q))
                    .sort((a, b) => {
                      const rank = (name: string) =>
                        name.toLowerCase() === q ? 0 : name.toLowerCase().startsWith(q) ? 1 : 2;
                      return rank(a) - rank(b) || a.localeCompare(b);
                    })
                : modelList;
              if (hits.length === 0) return null;
              return createPortal(
                <div
                  ref={modelMenuRef}
                  className={styles.modelSuggestMenu}
                  role="listbox"
                  style={modelMenuStyle ?? { visibility: "hidden" }}
                >
                  {hits.slice(0, 100).map((m) => (
                    <button
                      key={m}
                      type="button"
                      role="option"
                      aria-selected={m === activeFields.model}
                      className={styles.modelSuggestOption}
                      onMouseDown={(e) => {
                        e.preventDefault();
                        updateFields({ model: m });
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
        {modelFetch === "ok" && <span className={styles.externalTestOk}>{modelFetchMsg}</span>}
        {modelFetch === "fail" && (
          <span className={styles.externalTestFail}>
            <XCircle size={13} strokeWidth={2} />
            {modelFetchMsg}
          </span>
        )}
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
              model: activeFields.model.trim() || "?",
            })}
            msg={textTest.chatMsg}
          />
        </div>
      ) : null}

      <div className={s.actions}>
        {editing ? null : (
          <Button variant="ghost" onClick={onClose}>
            {t("common.cancel")}
          </Button>
        )}
        <Button
          variant="outline"
          icon={Activity}
          onClick={() => void runApiTest()}
          loading={textTest.running}
        >
          {textTest.running
            ? t("aiSettings.external.testRunning")
            : t("settings.aiModels.cloud.test")}
        </Button>
        {editing ? (
          <Button variant="solid" tone="accent" onClick={onClose}>
            {t("settings.aiModels.cloud.done")}
          </Button>
        ) : (
          <Button
            variant="solid"
            tone="accent"
            onClick={saveAsConnection}
            disabled={
              !activeFields.endpoint.trim() || ai.cloud.length >= MAX_CONNECTIONS || sameAsAnySaved
            }
            title={t("aiSettings.external.saveProfileHint")}
          >
            {t("common.save")}
          </Button>
        )}
      </div>
    </div>
  );
}
