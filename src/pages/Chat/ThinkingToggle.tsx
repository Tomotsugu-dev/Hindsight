import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { Check, ChevronDown } from "lucide-react";
import { useSettings } from "../../state/settings";
import { chatUsesCloud } from "./chatRouting";
import styles from "./ChatPage.module.css";

/**
 * 各服务商真实存在的思考档位(与后端 chat::llm::inject_cloud_thinking 的
 * 值域一致,两边同步改;均经真机实测):
 * - deepseek:reasoning_effort low/high/max(官方无 medium);
 * - kimi(Moonshot)/openrouter:low/medium/high;
 * - openai:Chat Completions 里 function tools 与思考**不能共存**(实测
 *   gpt-5.6-luna 400),对话必带工具,所以只给「关闭」一档,不画饼;
 * - 本地与其它云端:只有开关(high 显示为「开启」)。
 */
function modesFor(provider: string | null): string[] {
  switch (provider) {
    case "deepseek":
      return ["auto", "low", "high", "max", "off"];
    case "kimi":
    case "kimi-cn":
    case "openrouter":
      return ["auto", "low", "medium", "high", "off"];
    case "openai":
      return ["off"];
    default:
      return ["auto", "high", "off"];
  }
}

/**
 * 思考模式下拉(ChatGPT 式,composer 内右侧):钮面只显示当前档文字,
 * 点开向上弹菜单,交互与 ModelBadge 同款。档位跟着当前服务商走,
 * 切换服务商后残留的值域外强度就近归到 high(后端注入同样就近降级)。
 * 值直接写 ai.chatThinking,后端 chat_ask 每次现读,无需其它接线。
 */
export default function ThinkingToggle() {
  const { t } = useTranslation();
  const { settings, update } = useSettings();
  const [open, setOpen] = useState(false);
  const wrapRef = useRef<HTMLSpanElement>(null);

  useEffect(() => {
    if (!open) return;
    const onDown = (e: MouseEvent) => {
      if (wrapRef.current && !wrapRef.current.contains(e.target as Node)) {
        setOpen(false);
      }
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") setOpen(false);
    };
    document.addEventListener("mousedown", onDown);
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("mousedown", onDown);
      document.removeEventListener("keydown", onKey);
    };
  }, [open]);

  if (!settings) return null;
  const ai = settings.ai;
  const provider = chatUsesCloud(ai) ? ai.externalProvider : null;
  const modes = modesFor(provider);
  const hasLevels = modes.length > 3;
  // 归一:换服务商后旧值可能不在新表里(值域外强度、只剩一档的 openai)。
  // 强度类残留就近取 high(与后端 effort_lmh 一致),否则退表内首项。
  const raw = ai.chatThinking;
  const mode = modes.includes(raw)
    ? raw
    : ["low", "medium", "high", "max", "on"].includes(raw) && modes.includes("high")
      ? "high"
      : modes.includes("auto")
        ? "auto"
        : modes[0];
  // 开关型服务商没有强度概念,"high" 用「开启」称呼
  const labelOf = (m: string) =>
    m === "high" && !hasLevels ? t("chat.thinkingMode.on") : t(`chat.thinkingMode.${m}`);
  const hint = hasLevels
    ? "chat.thinkingMode.hintLevels"
    : provider === "openai"
      ? "chat.thinkingMode.hintOpenai"
      : "chat.thinkingMode.hint";

  const select = (next: string) => {
    setOpen(false);
    if (next !== ai.chatThinking) update({ ai: { ...ai, chatThinking: next } });
  };

  return (
    <span ref={wrapRef} className={styles.thinkWrap}>
      <button
        type="button"
        className={styles.thinkBtn}
        title={t("chat.thinkingMode.label")}
        onClick={() => setOpen((v) => !v)}
        aria-haspopup="menu"
        aria-expanded={open}
        aria-label={t("chat.thinkingMode.label")}
      >
        {labelOf(mode)}
        <ChevronDown size={12} strokeWidth={2.2} />
      </button>

      {open && (
        <div className={`${styles.badgeMenu} ${styles.thinkMenu}`} role="menu">
          {modes.map((m) => (
            <button
              key={m}
              type="button"
              role="menuitem"
              className={styles.badgeMenuItem}
              onClick={() => select(m)}
            >
              <span className={styles.badgeMenuLabel}>{labelOf(m)}</span>
              {mode === m && <Check size={12} strokeWidth={2.4} />}
            </button>
          ))}
          <p className={styles.badgeMenuEmpty}>{t(hint)}</p>
        </div>
      )}
    </span>
  );
}
