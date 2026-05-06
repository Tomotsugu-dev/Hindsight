import { useEffect, useRef, useState, type FormEvent } from "react";
import {
  ArrowUp,
  BarChart3,
  Bot,
  RotateCcw,
  Search,
  Sparkles,
} from "lucide-react";
import styles from "./ChatTab.module.css";

/** 消息类型：用户消息 + 4 种助手回答（纯文本 / 搜历史命中 / 数据条 / 反思分段）。 */
type Message =
  | { id: string; role: "user"; text: string }
  | { id: string; role: "assistant"; kind: "text"; text: string }
  | { id: string; role: "assistant"; kind: "search"; intro: string; hits: SearchHit[] }
  | { id: string; role: "assistant"; kind: "stat"; intro: string; total: string; bars: StatBar[] }
  | { id: string; role: "assistant"; kind: "reflection"; sections: ReflectionSection[] };

interface SearchHit {
  date: string;
  range: string;
  segLabel: string;
  segColor: string;
  snippet: string;
}
interface StatBar {
  label: string;
  valueLabel: string;
  ratio: number; // 0..1
}
interface ReflectionSection {
  heading: string;
  body: string;
}

/** 空状态下的快捷示例问题，点击直接发送，覆盖三种主要场景。 */
const PRESETS: Array<{
  icon: typeof Search;
  label: string;
  q: string;
}> = [
  {
    icon: Search,
    label: "搜历史",
    q: "找那个 PairingSection 的 hover bug 是哪天调的",
  },
  {
    icon: BarChart3,
    label: "查数据",
    q: "上周我在 Cursor 用了多久？",
  },
  {
    icon: Sparkles,
    label: "反思",
    q: "今天我专注度如何？给点反思建议",
  },
];

function uid() {
  return Math.random().toString(36).slice(2, 9);
}

/** mock 路由：按关键词返回不同形态的回答，演示三种交互范式。 */
function mockReply(q: string): Message[] {
  const userMsg: Message = { id: uid(), role: "user", text: q };

  if (/搜|找|什么时候|哪天|hover|bug|当时/.test(q)) {
    return [
      userMsg,
      {
        id: uid(),
        role: "assistant",
        kind: "search",
        intro: "在历史段总结里找到 3 处相关：",
        hits: [
          {
            date: "2026-05-06",
            range: "14:00–17:00",
            segLabel: "下午",
            segColor: "#fbbf24",
            snippet:
              "在 Hindsight 项目里调 PairingSection 的 hover/glow 效果，反复试 grid template 固定列宽 vs minmax，最后用 :has(.nameCol:hover) 让整列展开。",
          },
          {
            date: "2026-05-05",
            range: "21:00–24:00",
            segLabel: "晚",
            segColor: "#a78bfa",
            snippet:
              "继续修 actionCol 左对齐 + padding-left 的问题。发现 1fr nameCol 会自动收缩抵消 padding，最终改 grid template 为固定 140px。",
          },
          {
            date: "2026-05-05",
            range: "16:00–19:00",
            segLabel: "下午",
            segColor: "#fbbf24",
            snippet:
              "第一次注意到 trigger 没对齐到同一条 X 线，怀疑是 grid 子像素计算或 flex justify-end 导致的。",
          },
        ],
      },
    ];
  }

  if (/多久|时长|时间|多长|花了|用了/.test(q)) {
    return [
      userMsg,
      {
        id: uid(),
        role: "assistant",
        kind: "stat",
        intro: "上周 Cursor 共 12h 23m，按天分布：",
        total: "12h 23m",
        bars: [
          { label: "周一", valueLabel: "2h 30m", ratio: 0.85 },
          { label: "周二", valueLabel: "1h 50m", ratio: 0.62 },
          { label: "周三", valueLabel: "2h 05m", ratio: 0.71 },
          { label: "周四", valueLabel: "0h 45m", ratio: 0.25 },
          { label: "周五", valueLabel: "2h 50m", ratio: 0.96 },
          { label: "周六", valueLabel: "1h 23m", ratio: 0.46 },
          { label: "周日", valueLabel: "1h 00m", ratio: 0.34 },
        ],
      },
    ];
  }

  if (/反思|专注|今天|怎么样|建议|表现/.test(q)) {
    return [
      userMsg,
      {
        id: uid(),
        role: "assistant",
        kind: "reflection",
        sections: [
          {
            heading: "今日整体",
            body: "深度专注约 4 小时（上午 + 下午各 2 小时），其余时间在切换 / 沟通。整体专注度尚可，但下午有约 40 分钟应用切换频次偏高（28 次/小时）。",
          },
          {
            heading: "亮点",
            body: "下午 14:00–16:00 在 Cursor 持续 2 小时几乎无切换，是今天最 deep work 的一段。",
          },
          {
            heading: "改进建议",
            body: "晚上 19:00 之后社交类应用占比 60%。可以考虑把这部分时间推后到睡前 30 分钟内集中处理，给晚饭后留一段连续工作或阅读时间。",
          },
        ],
      },
    ];
  }

  return [
    userMsg,
    {
      id: uid(),
      role: "assistant",
      kind: "text",
      text:
        "（mock）这是一个示例回答。当前后端尚未接入，只识别几类关键词：\n• 搜历史 — 含「搜 / 找 / 什么时候 / hover / bug」\n• 查数据 — 含「多久 / 时长 / 花了」\n• 反思 — 含「反思 / 专注 / 今天 / 怎么样」",
    },
  ];
}

/**
 * 对话 tab：MVP 前端外壳。
 * - 空状态：欢迎语 + 三个快捷示例卡片，覆盖搜历史 / 查数据 / 反思三种典型问法。
 * - 有消息时：消息气泡列表（用户右、助手左），助手消息按 kind 渲染富内容。
 * - 底部输入框 + 发送按钮，类似 ChatGPT。
 *
 * 后端尚未接入 —— 当前用 mockReply 按关键词路由回不同形态的示例回答，展示交互骨架。
 */
export default function ChatTab() {
  const [messages, setMessages] = useState<Message[]>([]);
  const [input, setInput] = useState("");
  const listRef = useRef<HTMLDivElement>(null);

  // 新消息进来时滚到底部
  useEffect(() => {
    const el = listRef.current;
    if (!el) return;
    el.scrollTo({ top: el.scrollHeight, behavior: "smooth" });
  }, [messages]);

  const send = (q: string) => {
    const trimmed = q.trim();
    if (!trimmed) return;
    setMessages((prev) => [...prev, ...mockReply(trimmed)]);
    setInput("");
  };

  const onSubmit = (e: FormEvent) => {
    e.preventDefault();
    send(input);
  };

  const hasMessages = messages.length > 0;

  return (
    <>
      <div className={styles.subtitleRow}>
        <p className={styles.subtitle}>
          跟 AI 聊聊你的活动数据 — 搜历史、查应用时长、反思一天。
          <span className={styles.mockBadge}>MOCK</span>
        </p>
        {hasMessages && (
          <button
            type="button"
            className={styles.clearBtn}
            onClick={() => setMessages([])}
            title="清空当前对话"
          >
            <RotateCcw size={12} strokeWidth={2.2} />
            清空
          </button>
        )}
      </div>

      <div className={styles.body}>
        {hasMessages ? (
          <div ref={listRef} className={styles.messageList}>
            {messages.map((m) => (
              <MessageBubble key={m.id} m={m} />
            ))}
          </div>
        ) : (
          <div className={styles.empty}>
            <div className={styles.emptyHero}>
              <span className={styles.emptyHeroIcon} aria-hidden>
                <Bot size={22} strokeWidth={1.75} />
              </span>
              <h3 className={styles.emptyHeroTitle}>有什么想问的？</h3>
              <p className={styles.emptyHeroHint}>
                试试下面这些示例，或者直接在底下输入。
              </p>
            </div>
            <div className={styles.presets}>
              {PRESETS.map((p) => {
                const Icon = p.icon;
                return (
                  <button
                    key={p.label}
                    type="button"
                    className={styles.presetCard}
                    onClick={() => send(p.q)}
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
          placeholder="问点什么…"
          value={input}
          onChange={(e) => setInput(e.target.value)}
          autoFocus
        />
        <button
          type="submit"
          className={styles.composerSend}
          disabled={!input.trim()}
          aria-label="发送"
          title="发送（Enter）"
        >
          <ArrowUp size={16} strokeWidth={2.4} />
        </button>
      </form>
    </>
  );
}

function MessageBubble({ m }: { m: Message }) {
  if (m.role === "user") {
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
        {m.kind === "text" && <p className={styles.bubbleText}>{m.text}</p>}

        {m.kind === "search" && (
          <>
            <p className={styles.bubbleText}>{m.intro}</p>
            <div className={styles.searchHits}>
              {m.hits.map((h, i) => (
                <div key={i} className={styles.searchHit}>
                  <div className={styles.searchHitHead}>
                    <span
                      className={styles.searchHitChip}
                      style={{
                        background: h.segColor,
                        color: isLightHex(h.segColor) ? "#1c1c24" : "#fff",
                      }}
                    >
                      {h.segLabel}
                    </span>
                    <span className={styles.searchHitDate}>
                      {h.date} · {h.range}
                    </span>
                  </div>
                  <p className={styles.searchHitSnippet}>{h.snippet}</p>
                </div>
              ))}
            </div>
          </>
        )}

        {m.kind === "stat" && (
          <>
            <p className={styles.bubbleText}>{m.intro}</p>
            <div className={styles.statBars}>
              {m.bars.map((b, i) => (
                <div key={i} className={styles.statBarRow}>
                  <span className={styles.statBarLabel}>{b.label}</span>
                  <div className={styles.statBarTrack}>
                    <div
                      className={styles.statBarFill}
                      style={{ width: `${b.ratio * 100}%` }}
                    />
                  </div>
                  <span className={styles.statBarValue}>{b.valueLabel}</span>
                </div>
              ))}
            </div>
            <p className={styles.statTotal}>合计 {m.total}</p>
          </>
        )}

        {m.kind === "reflection" && (
          <div className={styles.reflection}>
            {m.sections.map((s, i) => (
              <div key={i} className={styles.reflectionSection}>
                <h4 className={styles.reflectionHeading}>{s.heading}</h4>
                <p className={styles.reflectionBody}>{s.body}</p>
              </div>
            ))}
          </div>
        )}
      </div>
    </div>
  );
}

/** 按 perceived luminance 判 hex 是否浅色——用来给段 chip 自动选黑/白文字。 */
function isLightHex(hex: string): boolean {
  const m = hex.match(/^#([0-9a-f]{3}|[0-9a-f]{6})$/i);
  if (!m) return true;
  let h = m[1];
  if (h.length === 3) h = h.split("").map((c) => c + c).join("");
  const r = parseInt(h.slice(0, 2), 16);
  const g = parseInt(h.slice(2, 4), 16);
  const b = parseInt(h.slice(4, 6), 16);
  return (0.299 * r + 0.587 * g + 0.114 * b) / 255 > 0.6;
}
