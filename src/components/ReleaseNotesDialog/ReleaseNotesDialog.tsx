import { useEffect, useRef } from "react";
import { createPortal } from "react-dom";
import { useTranslation } from "react-i18next";
import { useFocusTrap } from "../../hooks/useFocusTrap";
import { MarkdownText } from "../MarkdownText/MarkdownText";
// Same look as ConfirmDialog, so reuse its styles to keep dialogs consistent.
import styles from "../ConfirmDialog/ConfirmDialog.module.css";

interface ReleaseNotesDialogProps {
  open: boolean;
  title: string;
  /** Markdown already picked for the UI language; null while loading. */
  text: string | null;
  onClose: () => void;
}

/** Read-only release notes dialog with a single Close button.
 *
 * Not built on ConfirmDialog: that one is confirm/cancel and treats any Enter on `document` as
 * confirm. Here Enter is left to the focused button (autoFocus is on Close).
 */
export function ReleaseNotesDialog({ open, title, text, onClose }: ReleaseNotesDialogProps) {
  const { t } = useTranslation();
  const dialogRef = useRef<HTMLDivElement>(null);
  useFocusTrap(open, dialogRef);

  useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, [open, onClose]);

  if (!open) return null;

  return createPortal(
    <div className={styles.backdrop} onMouseDown={onClose} role="presentation">
      {/* eslint-disable-next-line jsx-a11y/no-noninteractive-element-interactions */}
      <div
        ref={dialogRef}
        className={styles.dialog}
        role="dialog"
        aria-modal="true"
        aria-labelledby="release-notes-title"
        onMouseDown={(e) => e.stopPropagation()}
      >
        <h2 id="release-notes-title" className={styles.title}>
          {title}
        </h2>
        <div className={`${styles.message} ${styles.messageRich}`}>
          {text === null ? null : <MarkdownText text={text} />}
        </div>
        <div className={styles.actions}>
          <button
            type="button"
            className={`${styles.btn} ${styles.btnConfirm}`}
            onClick={onClose}
            // eslint-disable-next-line jsx-a11y/no-autofocus
            autoFocus
          >
            {t("common.close")}
          </button>
        </div>
      </div>
    </div>,
    document.body,
  );
}
