import { useEffect, useRef } from "react";
import { createPortal } from "react-dom";
import { useTranslation } from "react-i18next";
import { TriangleAlert } from "lucide-react";
import { useFocusTrap } from "../../hooks/useFocusTrap";
import styles from "./SyncOptInDialog.module.css";

interface SyncOptInDialogProps {
  open: boolean;
  /** 数据集名(已本地化,如「屏幕记忆全文」) */
  datasetLabel: string;
  /** 该数据集的风险说明正文 */
  body: string;
  onConfirm: () => void;
  onCancel: () => void;
}

/**
 * 可选上云开关的确认弹窗:琥珀警告色,明示该数据集将上传到用户的
 * Google Drive(appData)。每次开启都弹——上云是显式决定,不做"记住"。
 */
export function SyncOptInDialog({
  open,
  datasetLabel,
  body,
  onConfirm,
  onCancel,
}: SyncOptInDialogProps) {
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

  return createPortal(
    <div className={styles.backdrop} onMouseDown={onCancel} role="presentation">
      {/* role="alertdialog" 已是 interactive role，但 ESLint plugin 仍按 div 默认判定 */}
      {/* eslint-disable-next-line jsx-a11y/no-noninteractive-element-interactions */}
      <div
        ref={dialogRef}
        className={styles.dialog}
        role="alertdialog"
        aria-modal="true"
        aria-labelledby="sync-optin-title"
        onMouseDown={(e) => e.stopPropagation()}
      >
        <h2 id="sync-optin-title" className={styles.title}>
          <TriangleAlert size={16} strokeWidth={2.2} className={styles.titleIcon} />
          {t("devices.cloud.datasets.confirmTitle", { dataset: datasetLabel })}
        </h2>
        <p className={styles.body}>{body}</p>
        <p className={styles.note}>{t("devices.cloud.datasets.confirmNote")}</p>
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
            {t("devices.cloud.datasets.confirmAction")}
          </button>
        </div>
      </div>
    </div>,
    document.body,
  );
}
