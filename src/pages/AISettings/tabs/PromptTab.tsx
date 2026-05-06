import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { MessageSquareText, RotateCcw, Save, User } from "lucide-react";
import { Section } from "../../../components/FormLayout/Section";
import { Row } from "../../../components/FormLayout/Row";
import { type PromptLanguage, type PromptOverrides } from "../../../api/hindsight";
import { DEFAULT_SYSTEM_PROMPTS, overrideKey } from "../../../lib/aiPrompts";
import { useAiSettings } from "../shared/useAiSettings";
import styles from "../AISettings.module.css";

export default function PromptTab() {
  const { t } = useTranslation();
  const { ai, updateAi } = useAiSettings();
  if (!ai) return null;

  return (
    <div className={styles.content}>
      <Section
        title={t("aiSettings.prompt.sectionTitle")}
        icon={MessageSquareText}
        description={t("aiSettings.prompt.sectionDesc")}
      >
        <PromptSection
          language={ai.promptLanguage}
          overrides={ai.promptOverrides}
          onSaveOverride={(lang, text) =>
            updateAi({
              promptOverrides: {
                ...ai.promptOverrides,
                [overrideKey(lang)]: text,
              },
            })
          }
        />
      </Section>

      <Section
        title={t("aiSettings.brief.sectionTitle")}
        icon={User}
        info={t("aiSettings.brief.sectionInfo")}
      >
        {/* hover 整个 Row（含 label）或 focus textarea 时才展开 textarea。
            Row label 一直可见，避免折叠态用户看不出这块是什么。 */}
        <div className={styles.briefHover}>
          <Row label={t("aiSettings.brief.rowLabel")} block>
            <div className={styles.briefCell}>
              <textarea
                className={styles.textarea}
                value={ai.userBrief}
                onChange={(e) => updateAi({ userBrief: e.target.value })}
                placeholder={t("aiSettings.brief.placeholder")}
                rows={6}
              />
            </div>
          </Row>
        </div>
      </Section>

    </div>
  );
}

/**
 * AI 提示词编辑器（Phase 1B-γ+）。
 *
 * 三种语言各独立维护一份覆盖：用户切语言时不会丢之前在别的语言写的覆盖。
 * 编辑器有"未保存改动"指示——避免用户切语言 / 关页时无声丢失改动。
 *
 * 数据流：
 *   props.overrides[langKey] 非空 → 走覆盖；否则展示内置默认（DEFAULT_SYSTEM_PROMPTS）
 *   保存 → onSaveOverride(lang, text)；text="" 等价"删除覆盖"
 *   重置 → 把 textarea 填回内置默认（不主动保存——给用户审一眼再决定要不要落库）
 */
function PromptSection({
  language,
  overrides,
  onSaveOverride,
}: {
  /** 当前生效的语言；跟随应用全局 i18n 走。 */
  language: PromptLanguage;
  overrides: PromptOverrides;
  onSaveOverride: (lang: PromptLanguage, text: string) => void;
}) {
  const { t } = useTranslation();
  const persistedFor = (lang: PromptLanguage): string => {
    const ov = overrides[overrideKey(lang)];
    return ov.trim().length > 0 ? ov : DEFAULT_SYSTEM_PROMPTS[lang];
  };

  // textarea 草稿：language 变（i18n 切换）时同步重置成新语言的持久值
  const [draft, setDraft] = useState<string>(() => persistedFor(language));
  useEffect(() => {
    setDraft(persistedFor(language));
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [language]);

  const persisted = persistedFor(language);
  const isDirty = draft !== persisted;
  const hasOverride = overrides[overrideKey(language)].trim().length > 0;

  const handleReset = () => {
    setDraft(DEFAULT_SYSTEM_PROMPTS[language]);
  };

  const handleSave = () => {
    // draft 跟内置默认完全一致 → 存空字符串等价"删除覆盖"
    const text =
      draft.trim() === DEFAULT_SYSTEM_PROMPTS[language].trim() ? "" : draft;
    onSaveOverride(language, text);
  };

  return (
    <div className={styles.promptWrap}>
      <Row label={t("aiSettings.prompt.rowLabel")} block>
        {/* Row.control 默认是 row flex；用 promptStack 改成 column，
            让 textarea 和按钮行各占一行而不是挤在同一行 */}
        <div className={styles.promptStack}>
          <textarea
            className={styles.promptTextarea}
            value={draft}
            onChange={(e) => setDraft(e.target.value)}
            rows={14}
            spellCheck={false}
          />
          <div className={styles.promptActions}>
            <span className={styles.promptHint}>
              {isDirty
                ? t("aiSettings.prompt.hint.dirty")
                : hasOverride
                  ? t("aiSettings.prompt.hint.custom")
                  : t("aiSettings.prompt.hint.default")}
            </span>
            <button
              type="button"
              className={styles.promptResetBtn}
              onClick={handleReset}
              disabled={draft === DEFAULT_SYSTEM_PROMPTS[language]}
              title={t("aiSettings.prompt.actions.resetTooltip")}
            >
              <RotateCcw size={13} strokeWidth={2} />
              {t("aiSettings.prompt.actions.reset")}
            </button>
            <button
              type="button"
              className={styles.promptSaveBtn}
              onClick={handleSave}
              disabled={!isDirty}
              title={t("aiSettings.prompt.actions.saveTooltip")}
            >
              <Save size={13} strokeWidth={2} />
              {t("aiSettings.prompt.actions.save")}
            </button>
          </div>
        </div>
      </Row>
    </div>
  );
}
