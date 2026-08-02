import {
  useEffect,
  useMemo,
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
  Brain,
  ChartPie,
  Check,
  ChevronDown,
  ChevronLeft,
  ChevronRight,
  ChevronUp,
  Clock,
  Copy,
  Globe,
  History,
  Mail,
  MonitorPlay,
  Pencil,
  RefreshCw,
  Search,
  Square,
  TrendingUp,
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
import ThinkingToggle from "./ThinkingToggle";
import { buildThread, type Message } from "./thread";

/** 瞬态气泡:发送中的乐观提问 / 错误(不落库,重载即清)。 */
type TransientMsg = { id: string; kind: "user" | "error"; text: string };

interface PresetItem {
  icon: typeof Search;
  label: string;
  q: string;
}

/** 随机位候选池：第 6 张卡从这里抽（点了就有像样结果的问题）。 */
const PRESET_POOL = [
  { key: "mail", icon: Mail },
  { key: "searchkw", icon: Search },
  { key: "hours", icon: Clock },
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
 * 固定五张：今日回顾 / 浏览器 / 视频站（简中问 B 站、其余语言问 YouTube，
 * 文案由各语言文件自带）/ 时间分配（按分类）/ 每日趋势（逐日分桶）；
 * 第 6 张为随机位，进入空态时从候选池抽一张。
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
      icon: ChartPie,
      label: t("chat.presets.category.label"),
      q: t("chat.presets.category.q"),
    },
    {
      icon: TrendingUp,
      label: t("chat.presets.trend.label"),
      q: t("chat.presets.trend.q"),
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
  // 库真源(消息树全量)+ 分支选择 → 当前路径;瞬态气泡(乐观提问/错误)单放
  const [rows, setRows] = useState<ChatStoredMessage[]>([]);
  const [transient, setTransient] = useState<TransientMsg[]>([]);
  const [branchChoice, setBranchChoice] = useState<Record<string, string>>({});
  // 正在编辑的提问 guid(气泡原位变输入框)
  const [editingGuid, setEditingGuid] = useState<string | null>(null);
  const [input, setInput] = useState("");
  const [busy, setBusy] = useState(false);
  // 正在重试的回答组 id:该组原位显示生成动画(旧答案暂时撤下,答完回来),
  // 期间底部不再追加 typing 气泡
  const [regeneratingId, setRegeneratingId] = useState<string | null>(null);
  const thread = useMemo(() => buildThread(rows, branchChoice), [rows, branchChoice]);
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
    setRegeneratingId(null);
    setEditingGuid(null);
    setBranchChoice({});
    setTransient([]);
    if (conversationId === null) {
      setRows([]);
      setBusy(false);
      return;
    }
    api
      .chatGetMessages(conversationId)
      .then((fresh) => {
        if (loadSeq.current === seq) setRows(fresh);
      })
      .catch((e) => {
        if (loadSeq.current === seq) {
          setRows([]);
          setTransient([{ id: uid(), kind: "error", text: String(e) }]);
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
      setRegeneratingId(null);
      const seq = loadSeq.current;
      api
        .chatGetMessages(e.payload.conversationId)
        .then((fresh) => {
          if (loadSeq.current === seq) {
            setRows(fresh);
            setTransient([]);
          }
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
  }, [rows, transient, busy]);

  /** 发送。`parentOverride`:编辑分支用——"" = 挂会话根,guid = 挂该消息下;
   *  缺省挂当前路径叶子(在旧分支上续聊也因此挂对位置)。 */
  const send = async (q: string, parentOverride?: string) => {
    const trimmed = q.trim();
    if (!trimmed || busy) return;
    // 隐私门:未确认过则弹窗;取消时输入原样保留
    if (!(await ensurePrivacyAck())) return;

    const seq = loadSeq.current;
    const askId = uid();
    askIdRef.current = askId;
    setTransient((prev) => [...prev, { id: uid(), kind: "user", text: trimmed }]);
    setInput("");
    setBusy(true);
    try {
      // 界面语言随请求传给后端:回答跟随提问语言,界面语言兜底
      const parent = parentOverride ?? thread.leafGuid ?? undefined;
      const ans = await api.chatAsk(
        trimmed,
        conversationId,
        i18n.language,
        askId,
        parent,
      );
      if (loadSeq.current !== seq) return; // 期间切了会话,丢弃
      if (conversationId === null) {
        if (ans.cancelled) {
          setTransient([]);
          return; // 首问被停止:会话已建但不接管,列表刷新时自然出现
        }
        // 新会话:接管 activeId,prop 变化会触发上面的装载 effect 从库取全量
        onConversationCreated(ans.conversationId);
      } else {
        // 以库为准重载(提问与答案已落库;停止时是"有问无答",同样以库为准)
        const fresh = await api.chatGetMessages(conversationId);
        if (loadSeq.current === seq) {
          setRows(fresh);
          setTransient([]);
        }
        if (!ans.cancelled) onConversationTouched();
      }
    } catch (e) {
      if (loadSeq.current === seq) {
        setTransient((prev) => [...prev, { id: uid(), kind: "error", text: String(e) }]);
      }
    } finally {
      if (loadSeq.current === seq) {
        setBusy(false);
        askIdRef.current = null;
      }
    }
  };

  /** 进入/保存/取消提问编辑。保存 = 在被编辑提问的父节点下开新分支重问;
   *  分支选择回落默认(最新),新分支答完自动成为当前路径。 */
  const startEdit = (guid: string) => {
    if (!busy) setEditingGuid(guid);
  };
  const cancelEdit = () => setEditingGuid(null);
  const saveEdit = (m: Extract<Message, { role: "user" }>, text: string) => {
    setEditingGuid(null);
    setBranchChoice((prev) => {
      const next = { ...prev };
      delete next[m.parentKey];
      return next;
    });
    void send(text, m.parentKey);
  };
  /** 切换某个提问的编辑分支(其下整段对话随路径切换)。 */
  const switchBranch = (parentKey: string, guid: string) => {
    setBranchChoice((prev) => ({ ...prev, [parentKey]: guid }));
  };

  /** 重新回答最后一条提问:后端追加新版本(不删旧的),前端以库为准重载。
   *  期间该回答组原位显示生成动画(旧答案暂时撤下,与 ChatGPT 的重试一致)。 */
  const regenerate = async (groupId: string) => {
    if (busy || conversationId === null) return;
    if (!(await ensurePrivacyAck())) return;
    const seq = loadSeq.current;
    const askId = uid();
    askIdRef.current = askId;
    setRegeneratingId(groupId);
    setBusy(true);
    try {
      const ans = await api.chatRegenerate(
        conversationId,
        i18n.language,
        askId,
        thread.leafGuid ?? undefined,
      );
      if (loadSeq.current !== seq) return;
      if (ans.cancelled) return;
      const fresh = await api.chatGetMessages(conversationId);
      if (loadSeq.current === seq) {
        setRows(fresh);
        setTransient([]);
      }
      onConversationTouched();
    } catch (e) {
      if (loadSeq.current === seq) {
        setTransient((prev) => [...prev, { id: uid(), kind: "error", text: String(e) }]);
      }
    } finally {
      setRegeneratingId(null);
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

  const hasMessages = thread.messages.length > 0 || transient.length > 0;
  // 「重试」只挂在当前路径最后一个回答组上(中间的重答会让下游历史失效)
  let lastAssistantIdx = -1;
  for (let i = thread.messages.length - 1; i >= 0; i--) {
    if (thread.messages[i].role === "assistant") {
      lastAssistantIdx = i;
      break;
    }
  }
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
            {thread.messages.map((m, i) =>
              m.id === regeneratingId && m.role === "assistant" ? (
                // 重试中:旧答案原位撤下,显示生成动画,答完以库为准回来
                <TypingBubble key={m.id} t={t} />
              ) : (
                <MessageBubble
                  key={m.id}
                  m={m}
                  t={t}
                  busy={busy}
                  isLastAssistant={i === lastAssistantIdx}
                  editingGuid={editingGuid}
                  onRegenerate={() => void regenerate(m.id)}
                  onStartEdit={startEdit}
                  onCancelEdit={cancelEdit}
                  onSaveEdit={saveEdit}
                  onSwitchBranch={switchBranch}
                />
              ),
            )}
            {transient.map((tm) => (
              <TransientBubble key={tm.id} m={tm} t={t} />
            ))}
            {busy && regeneratingId === null && <TypingBubble t={t} />}
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
        <ThinkingToggle />
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

/** 生成中的打字指示气泡:列表尾部(新提问)与原位替换(重试)共用。 */
function TypingBubble({ t }: { t: TFunction }) {
  return (
    <div className={`${styles.bubbleRow} ${styles.bubbleRowAssistant}`}>
      <span className={styles.assistantAvatar} aria-hidden>
        <Bot size={13} strokeWidth={2} />
      </span>
      <div className={`${styles.bubble} ${styles.bubbleAssistant}`}>
        <span className={styles.typing} role="status" aria-label={t("chat.thinking")}>
          <span className={styles.typingDot} />
          <span className={styles.typingDot} />
          <span className={styles.typingDot} />
        </span>
      </div>
    </div>
  );
}

/** 复制按钮:成功后图标变 ✓ 1.5 秒。用户提问与助手回答共用。 */
function CopyButton({ text, t }: { text: string; t: TFunction }) {
  const [copied, setCopied] = useState(false);
  const copy = () => {
    void navigator.clipboard
      .writeText(text)
      .then(() => {
        setCopied(true);
        window.setTimeout(() => setCopied(false), 1500);
      })
      .catch(() => {});
  };
  return (
    <button
      type="button"
      className={styles.msgAction}
      onClick={copy}
      data-tip={t("chat.actions.copy")}
      aria-label={t("chat.actions.copy")}
    >
      {copied ? <Check size={14} strokeWidth={2.2} /> : <Copy size={14} strokeWidth={2.2} />}
    </button>
  );
}

/** 瞬态气泡:发送中的乐观提问(无操作行)/ 错误(不落库,切走即清)。 */
function TransientBubble({ m, t }: { m: TransientMsg; t: TFunction }) {
  if (m.kind === "user") {
    return (
      <div className={`${styles.bubbleRow} ${styles.bubbleRowUser}`}>
        <div className={`${styles.bubble} ${styles.bubbleUser}`}>{m.text}</div>
      </div>
    );
  }
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

function MessageBubble({
  m,
  t,
  busy,
  isLastAssistant,
  editingGuid,
  onRegenerate,
  onStartEdit,
  onCancelEdit,
  onSaveEdit,
  onSwitchBranch,
}: {
  m: Message;
  t: TFunction;
  busy: boolean;
  isLastAssistant: boolean;
  editingGuid: string | null;
  onRegenerate: () => void;
  onStartEdit: (guid: string) => void;
  onCancelEdit: () => void;
  onSaveEdit: (m: Extract<Message, { role: "user" }>, text: string) => void;
  onSwitchBranch: (parentKey: string, guid: string) => void;
}) {
  if (m.role === "user") {
    return (
      <UserBubble
        m={m}
        t={t}
        busy={busy}
        editing={editingGuid === m.guid}
        onStartEdit={onStartEdit}
        onCancelEdit={onCancelEdit}
        onSaveEdit={onSaveEdit}
        onSwitchBranch={onSwitchBranch}
      />
    );
  }
  return (
    <AssistantBubble
      m={m}
      t={t}
      busy={busy}
      isLast={isLastAssistant}
      onRegenerate={onRegenerate}
    />
  );
}

/** 提问气泡:操作行(分支切换 ‹n/n› / 编辑 / 复制);编辑态原位变输入框,
 *  保存 = 在同一父节点下开新分支重问(Claude 同款交互)。 */
function UserBubble({
  m,
  t,
  busy,
  editing,
  onStartEdit,
  onCancelEdit,
  onSaveEdit,
  onSwitchBranch,
}: {
  m: Extract<Message, { role: "user" }>;
  t: TFunction;
  busy: boolean;
  editing: boolean;
  onStartEdit: (guid: string) => void;
  onCancelEdit: () => void;
  onSaveEdit: (m: Extract<Message, { role: "user" }>, text: string) => void;
  onSwitchBranch: (parentKey: string, guid: string) => void;
}) {
  const [draft, setDraft] = useState(m.text);
  // 进入编辑态时以原文起草(上次未保存的草稿不留)
  useEffect(() => {
    if (editing) setDraft(m.text);
  }, [editing, m.text]);

  if (editing) {
    return (
      <div className={`${styles.bubbleRow} ${styles.bubbleRowUser}`}>
        <div className={styles.editBox}>
          <textarea
            className={styles.editInput}
            value={draft}
            rows={3}
            onChange={(e) => setDraft(e.target.value)}
            // eslint-disable-next-line jsx-a11y/no-autofocus
            autoFocus
          />
          <div className={styles.editFooter}>
            <span className={styles.editHint}>{t("chat.edit.hint")}</span>
            <div className={styles.editBtns}>
              <button type="button" className={styles.editCancel} onClick={onCancelEdit}>
                {t("chat.edit.cancel")}
              </button>
              <button
                type="button"
                className={styles.editSave}
                disabled={!draft.trim()}
                onClick={() => onSaveEdit(m, draft)}
              >
                {t("chat.edit.save")}
              </button>
            </div>
          </div>
        </div>
      </div>
    );
  }

  return (
    <div className={`${styles.bubbleRow} ${styles.bubbleRowUser}`}>
      <div className={styles.bubbleColUser}>
        <div className={`${styles.bubble} ${styles.bubbleUser}`}>{m.text}</div>
        <div className={styles.msgActionsUser}>
          {m.branchCount > 1 && (
            <span className={styles.versionNav}>
              <button
                type="button"
                className={styles.msgAction}
                onClick={() => onSwitchBranch(m.parentKey, m.siblings[m.branchIdx - 1])}
                disabled={m.branchIdx === 0}
                data-tip={t("chat.actions.prevBranch")}
                aria-label={t("chat.actions.prevBranch")}
              >
                <ChevronLeft size={14} strokeWidth={2.2} />
              </button>
              {m.branchIdx + 1}/{m.branchCount}
              <button
                type="button"
                className={styles.msgAction}
                onClick={() => onSwitchBranch(m.parentKey, m.siblings[m.branchIdx + 1])}
                disabled={m.branchIdx === m.branchCount - 1}
                data-tip={t("chat.actions.nextBranch")}
                aria-label={t("chat.actions.nextBranch")}
              >
                <ChevronRight size={14} strokeWidth={2.2} />
              </button>
            </span>
          )}
          <button
            type="button"
            className={styles.msgAction}
            onClick={() => onStartEdit(m.guid)}
            disabled={busy}
            data-tip={t("chat.actions.edit")}
            aria-label={t("chat.actions.edit")}
          >
            <Pencil size={14} strokeWidth={2.2} />
          </button>
          <CopyButton text={m.text} t={t} />
        </div>
      </div>
    </div>
  );
}

function AssistantBubble({
  m,
  t,
  busy,
  isLast,
  onRegenerate,
}: {
  m: Extract<Message, { role: "assistant" }>;
  t: TFunction;
  busy: boolean;
  isLast: boolean;
  onRegenerate: () => void;
}) {
  const versions = m.versions;
  // 版本指针默认最新;重新回答追加版本后自动跳到新版
  const [idx, setIdx] = useState(versions.length - 1);
  useEffect(() => {
    setIdx(versions.length - 1);
  }, [versions.length]);
  const v = versions[Math.min(idx, versions.length - 1)];

  return (
    <div className={`${styles.bubbleRow} ${styles.bubbleRowAssistant}`}>
      <span className={styles.assistantAvatar} aria-hidden>
        <Bot size={13} strokeWidth={2} />
      </span>
      <div className={styles.bubbleColAssistant}>
        <div className={`${styles.bubble} ${styles.bubbleAssistant}`}>
          <div className={styles.bubbleMd}>
            <ReactMarkdown remarkPlugins={[remarkGfm]}>{v.text}</ReactMarkdown>
          </div>
          {v.citations.length > 0 && <CitationList citations={v.citations} t={t} />}
          {v.promptTokens != null && v.completionTokens != null && (
            <div className={styles.tokenMeta}>
              <span data-tip={t("chat.tokens.prompt")}>
                <ArrowUp size={11} strokeWidth={2.2} />
                {v.promptTokens.toLocaleString()} tokens
              </span>
              <span data-tip={t("chat.tokens.completion")}>
                <ArrowDown size={11} strokeWidth={2.2} />
                {v.completionTokens.toLocaleString()} tokens
              </span>
              {/* 思考消耗已含在 completion 内,tooltip 里讲明白免得用户以为要相加 */}
              {v.reasoningTokens != null && v.reasoningTokens > 0 && (
                <span data-tip={t("chat.tokens.reasoning")}>
                  <Brain size={11} strokeWidth={2.2} />
                  {v.reasoningTokens.toLocaleString()} tokens
                </span>
              )}
              {v.elapsedMs != null && v.elapsedMs > 0 && (
                <span data-tip={t("chat.tokens.elapsed")}>
                  <Clock size={11} strokeWidth={2.2} />
                  {(v.elapsedMs / 1000).toFixed(1)}s
                </span>
              )}
            </div>
          )}
        </div>
        <div className={styles.msgActions}>
          {versions.length > 1 && (
            <span className={styles.versionNav}>
              <button
                type="button"
                className={styles.msgAction}
                onClick={() => setIdx((i) => Math.max(0, i - 1))}
                disabled={idx === 0}
                data-tip={t("chat.actions.prevVersion")}
                aria-label={t("chat.actions.prevVersion")}
              >
                <ChevronLeft size={14} strokeWidth={2.2} />
              </button>
              {idx + 1}/{versions.length}
              <button
                type="button"
                className={styles.msgAction}
                onClick={() => setIdx((i) => Math.min(versions.length - 1, i + 1))}
                disabled={idx === versions.length - 1}
                data-tip={t("chat.actions.nextVersion")}
                aria-label={t("chat.actions.nextVersion")}
              >
                <ChevronRight size={14} strokeWidth={2.2} />
              </button>
            </span>
          )}
          <CopyButton text={v.text} t={t} />
          {isLast && (
            <button
              type="button"
              className={styles.msgAction}
              onClick={onRegenerate}
              disabled={busy}
              data-tip={t("chat.actions.regenerate")}
              aria-label={t("chat.actions.regenerate")}
            >
              <RefreshCw size={14} strokeWidth={2.2} />
            </button>
          )}
        </div>
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
