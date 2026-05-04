import { useEffect, useRef, useState, type KeyboardEvent } from "react";
import { AppWindow, EyeOff, Globe, Plus, X } from "lucide-react";
import { Section } from "../components/Section";
import { Row } from "../components/Row";
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
            aria-label={`移除 ${kw}`}
            title="移除"
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
  const { settings, update } = useSettings();
  if (!settings) return null;

  const urlList = settings.privacyUrlKeywords ?? [];
  const appList = settings.privacyAppKeywords ?? [];

  return (
    <>
      <Section
        title="浏览器过滤"
        info="浏览器地址栏 URL 包含其中任意一条（忽略大小写）时跳过截图。"
        icon={Globe}
      >
        <Row label="URL 关键词" block>
          <KeywordEditor
            value={urlList}
            onChange={(next) => update({ privacyUrlKeywords: next })}
            addLabel="添加 URL 后缀"
            placeholder="如 /login"
          />
        </Row>
      </Section>

      <Section
        title="应用过滤"
        info="应用名或窗口标题包含其中任意一条（忽略大小写）时跳过截图。"
        icon={AppWindow}
      >
        <Row label="标题 / 应用关键词" block>
          <KeywordEditor
            value={appList}
            onChange={(next) => update({ privacyAppKeywords: next })}
            addLabel="添加应用名"
            placeholder="如 微信"
          />
        </Row>
      </Section>

      <Section title="作用范围" info="命中过滤条件时只跳过截图" icon={EyeOff}>
        <p className={styles.notice}>
          截图过滤是本地行为，
          <strong className={styles.attention}>
            对已经存下来的历史截图无效
          </strong>
          ；要清空历史截图请去
          <span className={styles.kbd}>设置 · 数据 · 清空截图</span>。
        </p>
      </Section>
    </>
  );
}
