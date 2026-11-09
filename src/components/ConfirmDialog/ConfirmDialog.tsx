import { useEffect, useRef, type ReactNode } from "react";
import { createPortal } from "react-dom";
import type { LucideIcon } from "lucide-react";
import { useFocusTrap } from "../../hooks/useFocusTrap";
import { useTranslation } from "react-i18next";
import { Button } from "../Button/Button";
import { ScrollIndicator } from "../ScrollIndicator/ScrollIndicator";
import styles from "./ConfirmDialog.module.css";

interface ConfirmDialogProps {
  open: boolean;
  title: string;
  centerTitle?: boolean;
  /** Shown under the title, and stays in view when the message scrolls */
  notice?: ReactNode;
  /** Optional body. Strings preserve line breaks; nodes can contain rich content. */
  message?: ReactNode;
  confirmLabel?: string;
  /** Shown before the confirm text. The spinner takes its place while busy */
  confirmIcon?: LucideIcon;
  cancelLabel?: string;
  variant?: "primary" | "danger";
  /** Grays out the confirm button, and Enter does nothing */
  confirmDisabled?: boolean;
  /** 正在执行：确认按钮转圈，两个按钮都不能点，弹窗关不掉 */
  busy?: boolean;
  onConfirm: () => void;
  onCancel: () => void;
}

export function ConfirmDialog({
  open,
  title,
  centerTitle = false,
  notice,
  message,
  confirmLabel,
  confirmIcon: ConfirmIcon,
  cancelLabel,
  variant = "primary",
  confirmDisabled = false,
  busy = false,
  onConfirm,
  onCancel,
}: ConfirmDialogProps) {
  const { t } = useTranslation();
  // 默认值在调用方未传入时回落到通用 i18n 文案
  const confirmText = confirmLabel ?? t("common.confirm");
  const cancelText = cancelLabel ?? t("common.cancel");
  useEffect(() => {
    if (!open || busy) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onCancel();
      if (e.key === "Enter" && !confirmDisabled) onConfirm();
    };
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, [open, busy, onCancel, onConfirm, confirmDisabled]);

  const dialogRef = useRef<HTMLDivElement>(null);
  useFocusTrap(open, dialogRef);
  const messageRef = useRef<HTMLDivElement>(null);

  if (!open) return null;

  return createPortal(
    <div className={styles.backdrop} onMouseDown={busy ? undefined : onCancel} role="presentation">
      {/* role="alertdialog" 已是 interactive role，但 ESLint plugin 仍按 div 默认判定 */}
      {/* eslint-disable-next-line jsx-a11y/no-noninteractive-element-interactions */}
      <div
        ref={dialogRef}
        className={styles.dialog}
        data-part="dialog"
        role="alertdialog"
        aria-modal="true"
        aria-labelledby="confirm-title"
        onMouseDown={(e) => e.stopPropagation()}
      >
        <h2
          id="confirm-title"
          className={`${styles.title} ${centerTitle ? styles.titleCenter : ""}`}
        >
          {title}
        </h2>
        {notice}
        {message != null && (
          <div className={styles.messageWrap}>
            <div ref={messageRef} className={styles.messageScroll}>
              {/* Rich content uses a div so block elements remain valid; strings preserve line breaks. */}
              {typeof message === "string" ? (
                <p className={styles.message}>{message}</p>
              ) : (
                <div className={`${styles.message} ${styles.messageRich}`}>{message}</div>
              )}
            </div>
            {/* The scrollbar of a long message is always in view, so the user knows to scroll */}
            <ScrollIndicator targetRef={messageRef} className={styles.messageTrack} />
          </div>
        )}
        <div className={styles.actions}>
          <Button disabled={busy} onClick={onCancel}>
            {cancelText}
          </Button>
          <Button
            variant="solid"
            tone={variant === "danger" ? "danger" : "accent"}
            icon={ConfirmIcon}
            loading={busy}
            disabled={confirmDisabled}
            onClick={onConfirm}
            // 弹窗打开时聚焦默认按钮符合 a11y 最佳实践（屏幕阅读器、键盘 user 都期望）
            // eslint-disable-next-line jsx-a11y/no-autofocus
            autoFocus
          >
            {confirmText}
          </Button>
        </div>
      </div>
    </div>,
    document.body,
  );
}
