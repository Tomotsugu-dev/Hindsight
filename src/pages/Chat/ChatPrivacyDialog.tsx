import { useEffect, useRef } from "react";
import { createPortal } from "react-dom";
import { useTranslation } from "react-i18next";
import { Cloud, HardDrive } from "lucide-react";
import { useFocusTrap } from "../../hooks/useFocusTrap";
import type { AiConfig } from "../../api/hindsight";
import { chatLocalModelName, chatUsesCloud } from "./chatRouting";
import styles from "./ChatPrivacyDialog.module.css";

interface ChatPrivacyDialogProps {
  open: boolean;
  ai: AiConfig;
  onConfirm: () => void;
  onCancel: () => void;
}

/**
 * Chat 首次发送前的隐私确认(仿 ConfirmDialog 骨架,正文是富内容):
 * - 路由行:云端 = 服务商 + 模型 ID,琥珀警告色;本地 = 模型文件名,普通色;
 * - 说明:发送内容 = 提问 + 命中的屏幕文字片段;云端时强调发往第三方。
 */
export function ChatPrivacyDialog({ open, ai, onConfirm, onCancel }: ChatPrivacyDialogProps) {
  const { t } = useTranslation();
  useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onCancel();
      if (e.key === "Enter") onConfirm();
    };
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, [open, onCancel, onConfirm]);

  const dialogRef = useRef<HTMLDivElement>(null);
  useFocusTrap(open, dialogRef);

  if (!open) return null;

  const cloud = chatUsesCloud(ai);
  // 服务商显示名走既有 i18n(aiSettings.external.provider.*),未知值回落原文
  const provider = t(`aiSettings.external.provider.${ai.externalProvider}`, {
    defaultValue: ai.externalProvider || ai.endpoint,
  });

  return createPortal(
    <div className={styles.backdrop} onMouseDown={onCancel} role="presentation">
      {/* role="alertdialog" 已是 interactive role，但 ESLint plugin 仍按 div 默认判定 */}
      {/* eslint-disable-next-line jsx-a11y/no-noninteractive-element-interactions */}
      <div
        ref={dialogRef}
        className={styles.dialog}
        role="alertdialog"
        aria-modal="true"
        aria-labelledby="chat-privacy-title"
        onMouseDown={(e) => e.stopPropagation()}
      >
        <h2 id="chat-privacy-title" className={styles.title}>
          {t("chat.privacy.title")}
        </h2>

        {cloud ? (
          <div className={`${styles.routeRow} ${styles.routeCloud}`}>
            <Cloud size={14} strokeWidth={2} />
            {t("chat.privacy.routeCloud", { provider, model: ai.model })}
          </div>
        ) : (
          <div className={styles.routeRow}>
            <HardDrive size={14} strokeWidth={2} />
            {t("chat.privacy.routeLocal", {
              model: chatLocalModelName(ai) || "?",
            })}
          </div>
        )}

        <p className={styles.body}>{t("chat.privacy.body")}</p>
        {cloud && <p className={styles.bodyCloud}>{t("chat.privacy.bodyCloud")}</p>}

        <div className={styles.actions}>
          <button
            type="button"
            className={`${styles.btn} ${styles.btnCancel}`}
            onClick={onCancel}
          >
            {t("common.cancel")}
          </button>
          <button
            type="button"
            className={`${styles.btn} ${styles.btnConfirm}`}
            onClick={onConfirm}
            // 弹窗打开时聚焦默认按钮符合 a11y 最佳实践
            // eslint-disable-next-line jsx-a11y/no-autofocus
            autoFocus
          >
            {t("chat.privacy.confirm")}
          </button>
        </div>
      </div>
    </div>,
    document.body,
  );
}
