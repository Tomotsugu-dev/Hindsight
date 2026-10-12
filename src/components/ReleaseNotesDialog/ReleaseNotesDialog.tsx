import { useEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { useTranslation } from "react-i18next";
import { useFocusTrap } from "../../hooks/useFocusTrap";
import {
  BUNDLED_VERSIONS,
  dropVersionLine,
  loadBundledReleaseNotes,
} from "../../lib/bundledReleaseNotes";
import { pickReleaseNotesForLang } from "../../lib/releaseNotes";
import { MarkdownText } from "../MarkdownText/MarkdownText";
// Title, notes text and buttons look the same as ConfirmDialog, so reuse its styles.
import confirmStyles from "../ConfirmDialog/ConfirmDialog.module.css";
import styles from "./ReleaseNotesDialog.module.css";

interface ReleaseNotesDialogProps {
  /** Selected when the dialog opens. Falls back to the newest bundled version. */
  version: string;
  onClose: () => void;
}

/** Read-only release notes: bundled versions on the left, the selected one's notes on the right.
 *
 * Mount it only while open, so each open starts again at `version`.
 *
 * Not built on ConfirmDialog: that one is confirm/cancel and treats any Enter on `document` as
 * confirm. Here Enter is left to the focused button (autoFocus is on Close).
 */
export function ReleaseNotesDialog({ version, onClose }: ReleaseNotesDialogProps) {
  const { t, i18n } = useTranslation();
  const dialogRef = useRef<HTMLDivElement>(null);
  const notesRef = useRef<HTMLDivElement>(null);
  const [selected, setSelected] = useState(() =>
    BUNDLED_VERSIONS.includes(version) ? version : (BUNDLED_VERSIONS[0] ?? ""),
  );
  // null while loading.
  const [text, setText] = useState<string | null>(null);
  useFocusTrap(true, dialogRef);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, [onClose]);

  useEffect(() => {
    // Clicking through versions quickly: only the last one's result is shown.
    let stale = false;
    const empty = t("settings.about.update.changelogEmpty");
    setText(null);
    notesRef.current?.scrollTo({ top: 0 });
    loadBundledReleaseNotes(selected)
      .then((raw) => {
        if (stale) return;
        setText(
          raw ? dropVersionLine(pickReleaseNotesForLang(raw, i18n.language), selected) : empty,
        );
      })
      .catch(() => {
        if (!stale) setText(empty);
      });
    return () => {
      stale = true;
    };
  }, [selected, i18n.language, t]);

  return createPortal(
    <div className={confirmStyles.backdrop} onMouseDown={onClose} role="presentation">
      {/* eslint-disable-next-line jsx-a11y/no-noninteractive-element-interactions */}
      <div
        ref={dialogRef}
        className={styles.dialog}
        role="dialog"
        aria-modal="true"
        aria-labelledby="release-notes-title"
        onMouseDown={(e) => e.stopPropagation()}
      >
        <h2 id="release-notes-title" className={confirmStyles.title}>
          {t("settings.about.update.changelogTitle")}
        </h2>
        <div className={styles.body}>
          <div className={styles.versions}>
            {BUNDLED_VERSIONS.map((v) => (
              <button
                key={v}
                type="button"
                className={`${styles.version} ${v === selected ? styles.versionActive : ""}`}
                aria-current={v === selected}
                onClick={() => setSelected(v)}
              >
                {v}
              </button>
            ))}
          </div>
          <div
            ref={notesRef}
            className={`${confirmStyles.message} ${confirmStyles.messageRich} ${styles.notes}`}
          >
            <h3 className={styles.notesVersion}>{selected}</h3>
            {text === null ? null : <MarkdownText text={text} />}
          </div>
        </div>
        <div className={confirmStyles.actions}>
          <button
            type="button"
            className={`${confirmStyles.btn} ${confirmStyles.btnConfirm}`}
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
