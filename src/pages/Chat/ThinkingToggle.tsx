import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { Check, ChevronDown } from "lucide-react";
import { useSettings } from "../../state/settings";
import {
  hasLevels,
  normalizeThinking,
  thinkingLabelKey,
  thinkingModes,
} from "../../lib/thinkingModes";
import { chatUsesCloud } from "./chatRouting";
import styles from "./ChatPage.module.css";

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
  // 对话总是发工具
  const modes = thinkingModes(provider, true);
  const mode = normalizeThinking(ai.chatThinking, modes);
  const labelOf = (m: string) => t(thinkingLabelKey(m, modes));
  const hint = hasLevels(modes)
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
