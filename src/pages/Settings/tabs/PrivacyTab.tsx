import { useEffect, useRef, useState, type KeyboardEvent } from "react";
import { useTranslation } from "react-i18next";
import { AppWindow, EyeOff, Globe, Plus, X } from "lucide-react";
import { Section } from "../../../components/FormLayout/Section";
import { Row } from "../../../components/FormLayout/Row";
import { useSettings } from "../../../state/settings";
import styles from "./PrivacyTab.module.css";

interface KeywordEditorProps {
  value: string[];
  onChange: (next: string[]) => void;
  /** "+ 添加 xxx" 按钮里的文案 */
  addLabel: string;
  /** 输入框 placeholder（提示形式举例） */
  placeholder?: string;
}

function KeywordEditor({
  value,
  onChange,
  addLabel,
  placeholder,
}: KeywordEditorProps) {
  const { t } = useTranslation();
  const [adding, setAdding] = useState(false);
  const [draft, setDraft] = useState("");
  const inputRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    if (adding) inputRef.current?.focus();
  }, [adding]);

  const commit = () => {
    const t = draft.trim();
    if (t && !value.includes(t)) {
      onChange([...value, t]);
    }
    setDraft("");
    setAdding(false);
  };

  const cancel = () => {
    setDraft("");
    setAdding(false);
  };

  const removeAt = (idx: number) => {
    onChange(value.filter((_, i) => i !== idx));
  };

  const onKeyDown = (e: KeyboardEvent<HTMLInputElement>) => {
    if (e.key === "Enter") {
      e.preventDefault();
      commit();
    } else if (e.key === "Escape") {
      cancel();
    } else if (e.key === "Backspace" && draft === "" && value.length > 0) {
      removeAt(value.length - 1);
    }
  };

  return (
    <div className={styles.list}>
      {value.map((kw, idx) => (
        <span key={`${kw}-${idx}`} className={styles.chip}>
          <span className={styles.chipText}>{kw}</span>
          <button
            type="button"
            className={styles.chipRemove}
            onClick={() => removeAt(idx)}
            aria-label={t("settings.privacy.keywordEditor.removeAria", {
              keyword: kw,
            })}
            title={t("settings.privacy.keywordEditor.removeTooltip")}
          >
            <X size={10} strokeWidth={2.25} />
          </button>
        </span>
      ))}
      {adding ? (
        <input
          ref={inputRef}
          className={styles.addInput}
          placeholder={placeholder}
          value={draft}
          maxLength={128}
          onChange={(e) => setDraft(e.target.value)}
          onBlur={commit}
          onKeyDown={onKeyDown}
          spellCheck={false}
        />
      ) : (
        <button
          type="button"
          className={styles.addBtn}
          onClick={() => setAdding(true)}
        >
          <Plus size={11} strokeWidth={2} />
          {addLabel}
        </button>
      )}
    </div>
  );
}

export default function PrivacyTab() {
  const { t } = useTranslation();
  const { settings, update } = useSettings();
  if (!settings) return null;

  const urlList = settings.privacyUrlKeywords ?? [];
  const appList = settings.privacyAppKeywords ?? [];

  return (
    <>
      <Section
        title={t("settings.privacy.browser.title")}
        info={t("settings.privacy.browser.info")}
        icon={Globe}
      >
        <Row label={t("settings.privacy.browser.rowLabel")} block>
          <KeywordEditor
            value={urlList}
            onChange={(next) => update({ privacyUrlKeywords: next })}
            addLabel={t("settings.privacy.browser.addLabel")}
            placeholder={t("settings.privacy.browser.placeholder")}
          />
        </Row>
      </Section>

      <Section
        title={t("settings.privacy.app.title")}
        info={t("settings.privacy.app.info")}
        icon={AppWindow}
      >
        <Row label={t("settings.privacy.app.rowLabel")} block>
          <KeywordEditor
            value={appList}
            onChange={(next) => update({ privacyAppKeywords: next })}
            addLabel={t("settings.privacy.app.addLabel")}
            placeholder={t("settings.privacy.app.placeholder")}
          />
        </Row>
      </Section>

      <Section
        title={t("settings.privacy.scope.title")}
        info={t("settings.privacy.scope.info")}
        icon={EyeOff}
      >
        <p className={styles.notice}>
          {t("settings.privacy.scope.noticePrefix")}
          <strong className={styles.attention}>
            {t("settings.privacy.scope.noticeEmph")}
          </strong>
          {t("settings.privacy.scope.noticeMiddle")}
          <span className={styles.kbd}>
            {t("settings.privacy.scope.noticeKbd")}
          </span>
          {t("settings.privacy.scope.noticeSuffix")}
        </p>
      </Section>
    </>
  );
}
