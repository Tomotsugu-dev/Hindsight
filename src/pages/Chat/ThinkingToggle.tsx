import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { Check, ChevronDown } from "lucide-react";
import type { ReasoningLevel } from "../../api/hindsight";
import { useSettings } from "../../state/settings";
import { findAdvanced, patchAdvanced } from "../../state/reportModel";
import {
  hasLevels,
  normalizeThinking,
  thinkingLabelKey,
  thinkingModes,
} from "../../lib/thinkingModes";
import { chatCloudConnection, chatModel } from "./chatRouting";
import styles from "./ChatPage.module.css";

/**
 * 聊天的思考模式下拉：按「chat + 当前选中的模型」读写 `ai.advanced`。
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
  const activeCloud = chatCloudConnection(ai);
  const activeModel = chatModel(ai);
  const provider = activeCloud?.dialect ?? null;
  // 对话总是发工具
  const modes = thinkingModes(provider, true);
  const stored = activeModel
    ? (findAdvanced(ai, "chat", activeModel)?.reasoning ?? "auto")
    : "auto";
  const mode = normalizeThinking(stored, modes);
  const labelOf = (m: string) => t(thinkingLabelKey(m, modes));
  const hint = hasLevels(modes)
    ? "chat.thinkingMode.hintLevels"
    : provider === "openai"
      ? "chat.thinkingMode.hintOpenai"
      : "chat.thinkingMode.hint";

  const select = (next: string) => {
    setOpen(false);
    if (!activeModel || next === stored) return;
    update({
      ai: {
        ...ai,
        advanced: patchAdvanced(ai, "chat", activeModel, {
          reasoning: next === "auto" ? null : (next as ReasoningLevel),
        }),
      },
    });
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
