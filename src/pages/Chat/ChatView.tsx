import { useEffect, useRef, useState, type FormEvent } from "react";
import { useTranslation } from "react-i18next";
import type { TFunction } from "i18next";
import {
  ArrowUp,
  BarChart3,
  Bot,
  ChevronDown,
  ChevronUp,
  History,
  Search,
} from "lucide-react";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import {
  api,
  type ChatCitation,
  type ChatStoredMessage,
} from "../../api/hindsight";
import styles from "./ChatView.module.css";

/** 消息：用户问题 / 助手回答（含证据引用）/ 出错（仅本地瞬态，不落库）。 */
type Message =
  | { id: string; role: "user"; text: string }
  | {
      id: string;
      role: "assistant";
      text: string;
      citations: ChatCitation[];
      degraded: boolean;
    }
  | { id: string; role: "error"; text: string };

interface PresetItem {
  icon: typeof Search;
  label: string;
  q: string;
}

/** 空状态下的快捷示例问题，点击直接发送，覆盖三种工具场景。 */
function buildPresets(t: TFunction): PresetItem[] {
  return [
    {
      icon: Search,
      label: t("chat.presets.search.label"),
      q: t("chat.presets.search.q"),
    },
    {
      icon: BarChart3,
      label: t("chat.presets.stat.label"),
      q: t("chat.presets.stat.q"),
    },
    {
      icon: History,
      label: t("chat.presets.timeline.label"),
      q: t("chat.presets.timeline.q"),
    },
  ];
}

function uid() {
  return Math.random().toString(36).slice(2, 9);
}

/** RFC3339 "2026-07-05T14:03:22+09:00" → "2026-07-05 14:03"；异常格式原样返回。 */
function fmtTs(ts: string): string {
  return ts.length >= 16 ? ts.slice(0, 16).replace("T", " ") : ts;
}

function storedToMessage(m: ChatStoredMessage): Message {
  if (m.role === "user") {
    return { id: `db-${m.id}`, role: "user", text: m.content };
  }
  return {
    id: `db-${m.id}`,
    role: "assistant",
    text: m.content,
    citations: m.citations,
    degraded: m.degraded,
  };
}

interface ChatViewProps {
  /** null = 尚未有会话（首条消息后由后端隐式创建） */
  conversationId: number | null;
  /** 首条消息后后端建了会话，通知父组件接管 activeId 并刷新列表 */
  onConversationCreated: (id: number) => void;
  /** 有新消息落库后（既有会话）通知父组件刷新列表排序 */
  onConversationTouched: () => void;
  /** 发送前的隐私门：false = 用户取消，本次不发送 */
  ensurePrivacyAck: () => Promise<boolean>;
}

/**
 * 聊天区：消息流 + 证据卡 + 输入框。
 * - 一问一答：后端跑完 agent 循环一次性返回；多轮历史由后端从库里读；
 * - 切换会话时从库装载消息；错误气泡只在本地 state，切走即消失。
 */
export default function ChatView({
  conversationId,
  onConversationCreated,
  onConversationTouched,
  ensurePrivacyAck,
}: ChatViewProps) {
  const { t } = useTranslation();
  const [messages, setMessages] = useState<Message[]>([]);
  const [input, setInput] = useState("");
  const [busy, setBusy] = useState(false);
  const listRef = useRef<HTMLDivElement>(null);
  // 发送中切换会话时，回包不再属于当前视图——用序号丢弃过期回包
  const loadSeq = useRef(0);

  // 切换会话 → 从库装载该会话消息;null → 空态
  useEffect(() => {
    const seq = ++loadSeq.current;
    if (conversationId === null) {
      setMessages([]);
      return;
    }
    api
      .chatGetMessages(conversationId)
      .then((rows) => {
        if (loadSeq.current === seq) setMessages(rows.map(storedToMessage));
      })
      .catch((e) => {
        if (loadSeq.current === seq) {
          setMessages([{ id: uid(), role: "error", text: String(e) }]);
        }
      });
  }, [conversationId]);

  // 新消息进来时滚到底部
  useEffect(() => {
    const el = listRef.current;
    if (!el) return;
    el.scrollTo({ top: el.scrollHeight, behavior: "smooth" });
  }, [messages, busy]);

  const send = async (q: string) => {
    const trimmed = q.trim();
    if (!trimmed || busy) return;
    // 隐私门:未确认过则弹窗;取消时输入原样保留
    if (!(await ensurePrivacyAck())) return;

    const seq = loadSeq.current;
    setMessages((prev) => [...prev, { id: uid(), role: "user", text: trimmed }]);
    setInput("");
    setBusy(true);
    try {
      const ans = await api.chatAsk(trimmed, conversationId);
      if (loadSeq.current !== seq) return; // 期间切了会话,丢弃
      setMessages((prev) => [
        ...prev,
        {
          id: uid(),
          role: "assistant",
          text: ans.text,
          citations: ans.citations,
          degraded: ans.degraded,
        },
      ]);
      if (conversationId === null) {
        onConversationCreated(ans.conversationId);
      } else {
        onConversationTouched();
      }
    } catch (e) {
      if (loadSeq.current === seq) {
        setMessages((prev) => [...prev, { id: uid(), role: "error", text: String(e) }]);
      }
    } finally {
      if (loadSeq.current === seq) setBusy(false);
    }
  };

  const onSubmit = (e: FormEvent) => {
    e.preventDefault();
    void send(input);
  };

  const hasMessages = messages.length > 0;
  const presets = buildPresets(t);

  return (
    <div className={styles.view}>
      <div className={styles.body}>
        {hasMessages ? (
          <div ref={listRef} className={styles.messageList}>
            {messages.map((m) => (
              <MessageBubble key={m.id} m={m} t={t} />
            ))}
            {busy && (
              <div className={`${styles.bubbleRow} ${styles.bubbleRowAssistant}`}>
                <span className={styles.assistantAvatar} aria-hidden>
                  <Bot size={13} strokeWidth={2} />
                </span>
                <div className={`${styles.bubble} ${styles.bubbleAssistant}`}>
                  <span
                    className={styles.typing}
                    role="status"
                    aria-label={t("chat.thinking")}
                  >
                    <span className={styles.typingDot} />
                    <span className={styles.typingDot} />
                    <span className={styles.typingDot} />
                  </span>
                </div>
              </div>
            )}
          </div>
        ) : (
          <div className={styles.empty}>
            <div className={styles.emptyHero}>
              <span className={styles.emptyHeroIcon} aria-hidden>
                <Bot size={22} strokeWidth={1.75} />
              </span>
              <h3 className={styles.emptyHeroTitle}>{t("chat.empty.title")}</h3>
              <p className={styles.emptyHeroHint}>{t("chat.empty.hint")}</p>
            </div>
            <div className={styles.presets}>
              {presets.map((p) => {
                const Icon = p.icon;
                return (
                  <button
                    key={p.label}
                    type="button"
                    className={styles.presetCard}
                    onClick={() => void send(p.q)}
                  >
                    <span className={styles.presetIcon}>
                      <Icon size={14} strokeWidth={2} />
                    </span>
                    <span className={styles.presetLabel}>{p.label}</span>
                    <span className={styles.presetQuestion}>{p.q}</span>
                  </button>
                );
              })}
            </div>
          </div>
        )}
      </div>

      <form className={styles.composer} onSubmit={onSubmit}>
        <input
          type="text"
          className={styles.composerInput}
          placeholder={t("chat.input.placeholder")}
          value={input}
          onChange={(e) => setInput(e.target.value)}
          disabled={busy}
          // 进入聊天页即可输入是用户预期，键盘 user 同样受益
          // eslint-disable-next-line jsx-a11y/no-autofocus
          autoFocus
        />
        <button
          type="submit"
          className={styles.composerSend}
          disabled={!input.trim() || busy}
          aria-label={t("chat.input.sendAria")}
          title={t("chat.input.sendTooltip")}
        >
          <ArrowUp size={16} strokeWidth={2.4} />
        </button>
      </form>
    </div>
  );
}

function MessageBubble({ m, t }: { m: Message; t: TFunction }) {
  if (m.role === "user") {
    return (
      <div className={`${styles.bubbleRow} ${styles.bubbleRowUser}`}>
        <div className={`${styles.bubble} ${styles.bubbleUser}`}>{m.text}</div>
      </div>
    );
  }

  if (m.role === "error") {
    return (
      <div className={`${styles.bubbleRow} ${styles.bubbleRowAssistant}`}>
        <span className={styles.assistantAvatar} aria-hidden>
          <Bot size={13} strokeWidth={2} />
        </span>
        <div className={`${styles.bubble} ${styles.bubbleAssistant}`}>
          <p className={styles.bubbleText}>{t("chat.error", { msg: m.text })}</p>
        </div>
      </div>
    );
  }

  return (
    <div className={`${styles.bubbleRow} ${styles.bubbleRowAssistant}`}>
      <span className={styles.assistantAvatar} aria-hidden>
        <Bot size={13} strokeWidth={2} />
      </span>
      <div className={`${styles.bubble} ${styles.bubbleAssistant}`}>
        <div className={styles.bubbleMd}>
          <ReactMarkdown remarkPlugins={[remarkGfm]}>{m.text}</ReactMarkdown>
        </div>
        {m.citations.length > 0 && <CitationList citations={m.citations} t={t} />}
      </div>
    </div>
  );
}

/** 证据卡超过这个数量时默认收起,只露前几条 */
const CITATIONS_COLLAPSED = 3;

function CitationList({ citations, t }: { citations: ChatCitation[]; t: TFunction }) {
  const [expanded, setExpanded] = useState(false);
  const collapsible = citations.length > CITATIONS_COLLAPSED + 1;
  const shown =
    collapsible && !expanded ? citations.slice(0, CITATIONS_COLLAPSED) : citations;

  return (
    <div className={styles.searchHits}>
      {shown.map((c) => (
        <div key={c.index} className={styles.searchHit}>
          <div className={styles.searchHitHead}>
            <span className={styles.searchHitChip}>[{c.index}]</span>
            <span className={styles.searchHitDate}>
              {fmtTs(c.startedTs)} – {fmtTs(c.endedTs).slice(-5)}
            </span>
          </div>
          <p className={styles.searchHitSnippet}>
            {c.app}
            {c.title ? ` · ${c.title}` : ""}
          </p>
        </div>
      ))}
      {collapsible && (
        <button
          type="button"
          className={styles.hitsToggle}
          onClick={() => setExpanded((v) => !v)}
        >
          {expanded ? (
            <>
              <ChevronUp size={12} strokeWidth={2.2} />
              {t("chat.citations.collapse")}
            </>
          ) : (
            <>
              <ChevronDown size={12} strokeWidth={2.2} />
              {t("chat.citations.showAll", { count: citations.length })}
            </>
          )}
        </button>
      )}
    </div>
  );
}
