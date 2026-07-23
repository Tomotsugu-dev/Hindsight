import {
  useEffect,
  useRef,
  useState,
  type FormEvent,
  type KeyboardEvent,
} from "react";
import { useTranslation } from "react-i18next";
import type { TFunction } from "i18next";
import { listen } from "@tauri-apps/api/event";
import {
  ArrowDown,
  ArrowUp,
  Bot,
  ChevronDown,
  ChevronUp,
  Globe,
  History,
  Mail,
  MonitorPlay,
  Search,
  Square,
} from "lucide-react";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import {
  api,
  CHAT_ANSWER_READY_EVENT,
  type ChatAnswerReadyPayload,
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
      /** 本轮上行/下行 token；旧数据为 null 不显示 */
      promptTokens: number | null;
      completionTokens: number | null;
    }
  | { id: string; role: "error"; text: string };

interface PresetItem {
  icon: typeof Search;
  label: string;
  q: string;
}

/** 随机位候选池：第 4 张卡从这里抽（都是"今天"口径、点了就有像样结果的问题）。 */
const PRESET_POOL = [
  { key: "mail", icon: Mail },
  { key: "searchkw", icon: Search },
] as const;

// 轮换游标（模块级：跨挂载/跨页面往返也接着轮）。起点随机，之后顺序轮换——
// 池子小，独立随机会频繁连抽同一张，看起来像"不更新"。
let poolCursor = Math.floor(Math.random() * PRESET_POOL.length);

function nextPoolPick(): (typeof PRESET_POOL)[number] {
  poolCursor = (poolCursor + 1) % PRESET_POOL.length;
  return PRESET_POOL[poolCursor];
}

/**
 * 空状态下的快捷示例问题，点击直接发送。
 * 固定三张：今日回顾 / 浏览器 / 视频站（简中问 B 站、其余语言问 YouTube，
 * 文案由各语言文件自带）；第 4 张为随机位，进入空态时从候选池抽一张。
 */
function buildPresets(t: TFunction, pool: (typeof PRESET_POOL)[number]): PresetItem[] {
  return [
    {
      icon: History,
      label: t("chat.presets.today.label"),
      q: t("chat.presets.today.q"),
    },
    {
      icon: Globe,
      label: t("chat.presets.browser.label"),
      q: t("chat.presets.browser.q"),
    },
    {
      icon: MonitorPlay,
      label: t("chat.presets.video.label"),
      q: t("chat.presets.video.q"),
    },
    {
      icon: pool.icon,
      label: t(`chat.presets.${pool.key}.label`),
      q: t(`chat.presets.${pool.key}.q`),
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
    promptTokens: m.promptTokens,
    completionTokens: m.completionTokens,
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
  const { t, i18n } = useTranslation();
  const [messages, setMessages] = useState<Message[]>([]);
  const [input, setInput] = useState("");
  const [busy, setBusy] = useState(false);
  const listRef = useRef<HTMLDivElement>(null);
  // 发送中切换会话时，回包不再属于当前视图——用序号丢弃过期回包
  const loadSeq = useRef(0);
  // 当前生成中问答的取消句柄(自己发起的,或重开会话时从后端注册表恢复的)
  const askIdRef = useRef<string | null>(null);
  // answer-ready 事件与 chatInflight 查询的竞态围栏:事件清掉 busy 后,
  // 迟到的 inflight 回包不允许再把 busy 置回 true
  const inflightEpoch = useRef(0);
  // 事件监听器闭包里读当前会话 id 用(监听器只注册一次)
  const convIdRef = useRef(conversationId);
  convIdRef.current = conversationId;

  // 切换会话 → 从库装载该会话消息;null → 空态。
  // 同时向后端注册表查"该会话是否正在生成"——跳页/关窗回来时恢复打字指示,
  // 也顺带修正旧版"发送中切会话导致 busy 永久卡死"的问题(busy 一律以查询为准)。
  useEffect(() => {
    const seq = ++loadSeq.current;
    const epoch = inflightEpoch.current;
    askIdRef.current = null;
    if (conversationId === null) {
      setMessages([]);
      setBusy(false);
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
    api
      .chatInflight(conversationId)
      .then((askId) => {
        if (loadSeq.current !== seq || inflightEpoch.current !== epoch) return;
        askIdRef.current = askId;
        setBusy(askId !== null);
      })
      .catch(() => {
        if (loadSeq.current === seq) setBusy(false);
      });
  }, [conversationId]);

  // 答案落库广播:当前会话命中 → 以库为准重载消息、清 busy。
  // 这是跳页/关窗后答案的唯一送达通道;自己 await 的路径同样"以库为准",双方幂等。
  useEffect(() => {
    const un = listen<ChatAnswerReadyPayload>(CHAT_ANSWER_READY_EVENT, (e) => {
      if (e.payload.conversationId !== convIdRef.current) return;
      inflightEpoch.current += 1;
      askIdRef.current = null;
      setBusy(false);
      const seq = loadSeq.current;
      api
        .chatGetMessages(e.payload.conversationId)
        .then((rows) => {
          if (loadSeq.current === seq) setMessages(rows.map(storedToMessage));
        })
        .catch(() => {});
    });
    return () => {
      void un.then((f) => f());
    };
  }, []);

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
    const askId = uid();
    askIdRef.current = askId;
    setMessages((prev) => [...prev, { id: uid(), role: "user", text: trimmed }]);
    setInput("");
    setBusy(true);
    try {
      // 界面语言随请求传给后端:回答跟随提问语言,界面语言兜底
      const ans = await api.chatAsk(trimmed, conversationId, i18n.language, askId);
      if (loadSeq.current !== seq) return; // 期间切了会话,丢弃
      if (ans.cancelled) return; // 点了停止:提问已入库,不渲染回答气泡
      if (conversationId === null) {
        // 新会话:接管 activeId,prop 变化会触发上面的装载 effect 从库取全量
        onConversationCreated(ans.conversationId);
      } else {
        // 以库为准重载(答案已落库;answer-ready 事件路径同样会刷,双方幂等)
        const rows = await api.chatGetMessages(conversationId);
        if (loadSeq.current === seq) setMessages(rows.map(storedToMessage));
        onConversationTouched();
      }
    } catch (e) {
      if (loadSeq.current === seq) {
        setMessages((prev) => [...prev, { id: uid(), role: "error", text: String(e) }]);
      }
    } finally {
      if (loadSeq.current === seq) {
        setBusy(false);
        askIdRef.current = null;
      }
    }
  };

  /** 停止当前生成:凭句柄取消,后端丢弃生成 future 并广播 ok=false。幂等。 */
  const stopGeneration = () => {
    const id = askIdRef.current;
    if (id) void api.chatCancel(id).catch(() => {});
  };

  const onSubmit = (e: FormEvent) => {
    e.preventDefault();
    void send(input);
  };

  /** Enter 发送、Shift+Enter 换行(textarea 默认行为);输入法组词中按下的
   *  Enter 是在确认候选词,不发送。 */
  const onInputKeyDown = (e: KeyboardEvent<HTMLTextAreaElement>) => {
    if (e.key === "Enter" && !e.shiftKey && !e.nativeEvent.isComposing) {
      e.preventDefault();
      void send(input);
    }
  };

  const hasMessages = messages.length > 0;
  // 随机位：挂载时抽一张，之后每次切换会话（含"新对话"）轮换到下一张；
  // 同一空态视图内保持稳定，不随输入重渲染跳变。
  const [poolPick, setPoolPick] = useState(nextPoolPick);
  const poolMounted = useRef(false);
  useEffect(() => {
    if (!poolMounted.current) {
      poolMounted.current = true; // 首次挂载已在 useState 初始化里抽过
      return;
    }
    setPoolPick(nextPoolPick());
  }, [conversationId]);
  const presets = buildPresets(t, poolPick);

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
        <textarea
          className={styles.composerInput}
          placeholder={t("chat.input.placeholder")}
          value={input}
          onChange={(e) => setInput(e.target.value)}
          onKeyDown={onInputKeyDown}
          disabled={busy}
          rows={1}
          // 进入聊天页即可输入是用户预期，键盘 user 同样受益
          // eslint-disable-next-line jsx-a11y/no-autofocus
          autoFocus
        />
        {busy ? (
          <button
            type="button"
            className={styles.composerSend}
            onClick={stopGeneration}
            aria-label={t("chat.input.stopAria")}
            title={t("chat.input.stopTooltip")}
          >
            <Square size={13} strokeWidth={2.4} fill="currentColor" />
          </button>
        ) : (
          <button
            type="submit"
            className={styles.composerSend}
            disabled={!input.trim()}
            aria-label={t("chat.input.sendAria")}
            title={t("chat.input.sendTooltip")}
          >
            <ArrowUp size={16} strokeWidth={2.4} />
          </button>
        )}
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
        {m.promptTokens != null && m.completionTokens != null && (
          <div className={styles.tokenMeta}>
            <span title={t("chat.tokens.prompt")}>
              <ArrowUp size={11} strokeWidth={2.2} />
              {m.promptTokens.toLocaleString()}
            </span>
            <span title={t("chat.tokens.completion")}>
              <ArrowDown size={11} strokeWidth={2.2} />
              {m.completionTokens.toLocaleString()}
            </span>
          </div>
        )}
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
