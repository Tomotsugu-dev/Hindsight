import { useEffect } from "react";
import { createPortal } from "react-dom";
import { useTranslation } from "react-i18next";
import styles from "./ConfirmDialog.module.css";

interface ConfirmDialogProps {
  open: boolean;
  title: string;
  message: string;
  confirmLabel?: string;
  cancelLabel?: string;
  variant?: "primary" | "danger";
  onConfirm: () => void;
  onCancel: () => void;
}

export function ConfirmDialog({
  open,
  title,
  message,
  confirmLabel,
  cancelLabel,
  variant = "primary",
  onConfirm,
  onCancel,
}: ConfirmDialogProps) {
  const { t } = useTranslation();
  // 默认值在调用方未传入时回落到通用 i18n 文案
  const confirmText = confirmLabel ?? t("common.confirm");
  const cancelText = cancelLabel ?? t("common.cancel");
  useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onCancel();
      if (e.key === "Enter") onConfirm();
    };
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, [open, onCancel, onConfirm]);

  if (!open) return null;

  return createPortal(
    <div className={styles.backdrop} onMouseDown={onCancel} role="presentation">
      <div
        className={styles.dialog}
        role="alertdialog"
        aria-modal="true"
        aria-labelledby="confirm-title"
        onMouseDown={(e) => e.stopPropagation()}
      >
        <h2 id="confirm-title" className={styles.title}>
          {title}
        </h2>
        <p className={styles.message}>{message}</p>
        <div className={styles.actions}>
          <button type="button" className={`${styles.btn} ${styles.btnCancel}`} onClick={onCancel}>
            {cancelText}
          </button>
          <button
            type="button"
            className={`${styles.btn} ${variant === "danger" ? styles.btnDanger : styles.btnConfirm}`}
            onClick={onConfirm}
            autoFocus
          >
            {confirmText}
          </button>
        </div>
      </div>
    </div>,
    document.body,
  );
}
