import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import ctl from "./ReportControls.module.css";
import styles from "./SummaryEditor.module.css";

interface SummaryEditorProps {
  /** 打开时框里的文字（Markdown 原文） */
  initial: string;
  /** 保存。出错时由调用方显示错误，编辑框保持打开 */
  onSave: (text: string) => Promise<void>;
  onCancel: () => void;
}

/** 日报的一段、周报的整周回顾共用的编辑框：改 Markdown 原文，保存或取消。
 *  Esc 取消，⌘/Ctrl + Enter 保存。 */
export function SummaryEditor({ initial, onSave, onCancel }: SummaryEditorProps) {
  const { t } = useTranslation();
  const [text, setText] = useState(initial);
  const [saving, setSaving] = useState(false);
  const ref = useRef<HTMLTextAreaElement>(null);
  const canSave = !saving && text.trim() !== "" && text !== initial;
  // 框的高度跟着原文行数走，长日报不用一打开就拖
  const rows = Math.min(24, Math.max(6, initial.split("\n").length + 2));

  useEffect(() => {
    const el = ref.current;
    if (!el) return;
    el.focus();
    el.setSelectionRange(el.value.length, el.value.length);
  }, []);

  const save = async () => {
    if (!canSave) return;
    setSaving(true);
    try {
      await onSave(text);
    } finally {
      setSaving(false);
    }
  };

  return (
    <div className={styles.editor}>
      <textarea
        ref={ref}
        className={styles.textarea}
        value={text}
        rows={rows}
        aria-label={t("aiSummary.edit.textareaLabel")}
        disabled={saving}
        onChange={(e) => setText(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === "Escape") {
            e.preventDefault();
            onCancel();
          } else if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) {
            e.preventDefault();
            void save();
          }
        }}
      />
      <div className={styles.footer}>
        <p className={styles.note}>{t("aiSummary.edit.note")}</p>
        <button type="button" className={ctl.textBtn} onClick={onCancel} disabled={saving}>
          {t("common.cancel")}
        </button>
        <button
          type="button"
          className={`${ctl.btn} ${ctl.btnPrimary} ${styles.saveBtn}`}
          onClick={() => void save()}
          disabled={!canSave}
        >
          {saving ? t("aiSummary.edit.saving") : t("common.save")}
        </button>
      </div>
    </div>
  );
}
