import {
  useEffect,
  useMemo,
  useRef,
  useState,
  type KeyboardEvent,
} from "react";
import { createPortal } from "react-dom";
import { useTranslation } from "react-i18next";
import { AppWindow, EyeOff, Globe, Plus, X } from "lucide-react";
import { Section } from "../../../components/FormLayout/Section";
import { Row } from "../../../components/FormLayout/Row";
import { AppIcon } from "../../../components/AppIcon/AppIcon";
import { useSettings } from "../../../state/settings";
import { api, type AppGroup } from "../../../api/hindsight";
import { logError } from "../../../lib/logger";
import {
  buildAppSuggestions,
  filterAppSuggestions,
  SUGGEST_MAX,
} from "../../../lib/appSuggest";
import styles from "./PrivacyTab.module.css";

interface KeywordEditorProps {
  value: string[];
  onChange: (next: string[]) => void;
  /** "+ 添加 xxx" 按钮里的文案 */
  addLabel: string;
  /** 输入框 placeholder（提示形式举例） */
  placeholder?: string;
  /** 开启应用名自动补全（候选来自真实采集过的进程）。URL 关键词那栏用不上。 */
  suggestApps?: boolean;
}

function KeywordEditor({
  value,
  onChange,
  addLabel,
  placeholder,
  suggestApps = false,
}: KeywordEditorProps) {
  const { t } = useTranslation();
  const [adding, setAdding] = useState(false);
  const [draft, setDraft] = useState("");
  const inputRef = useRef<HTMLInputElement>(null);
  // 自动补全：候选池展开输入框时懒加载一次；hi = 高亮下标（-1 表示无高亮，
  // 此时 Enter 提交输入原文——关键词是自由子串，候选之外的输入必须能提交）
  const [groups, setGroups] = useState<AppGroup[] | null>(null);
  const [hi, setHi] = useState(-1);
  // 面板 fixed 定位：设置页是滚动容器，absolute 面板会被裁（同分类页的处理）
  const [menuRect, setMenuRect] = useState<{
    top: number;
    left: number;
    width: number;
  } | null>(null);

  useEffect(() => {
    if (adding) inputRef.current?.focus();
  }, [adding]);

  useEffect(() => {
    if (!adding || !suggestApps) {
      setMenuRect(null);
      return;
    }
    if (groups === null) {
      api
        .listAppGroups()
        .then(setGroups)
        .catch((e) => {
          logError("privacy.appSuggest", e);
          setGroups([]); // 拉不到候选就退化为纯手输，不挡添加
        });
    }
    const measure = () => {
      const r = inputRef.current?.getBoundingClientRect();
      if (r) setMenuRect({ top: r.bottom + 4, left: r.left, width: r.width });
    };
    measure();
    window.addEventListener("scroll", measure, true);
    window.addEventListener("resize", measure);
    return () => {
      window.removeEventListener("scroll", measure, true);
      window.removeEventListener("resize", measure);
    };
  }, [adding, suggestApps, groups]);

  // 候选池 / 过滤与分类页共用同一套规则；已加过的关键词不再重复建议
  const matches = useMemo(() => {
    if (!suggestApps || !groups) return [];
    return filterAppSuggestions(
      buildAppSuggestions(groups, value),
      draft,
      SUGGEST_MAX,
    );
  }, [suggestApps, groups, value, draft]);

  // 输入变化后高亮回到"无"，避免指到过滤后错位的项
  useEffect(() => {
    setHi(-1);
  }, [draft]);

  const commitText = (text: string) => {
    const trimmed = text.trim();
    if (trimmed && !value.includes(trimmed)) {
      onChange([...value, trimmed]);
    }
    setDraft("");
    setAdding(false);
    setHi(-1);
  };

  // 高亮着候选就提交候选的进程名，否则提交输入原文
  const commit = () => commitText(hi >= 0 && matches[hi] ? matches[hi].process : draft);

  const cancel = () => {
    setDraft("");
    setAdding(false);
    setHi(-1);
  };

  const removeAt = (idx: number) => {
    onChange(value.filter((_, i) => i !== idx));
  };

  const onKeyDown = (e: KeyboardEvent<HTMLInputElement>) => {
    if (e.key === "ArrowDown" && matches.length > 0) {
      e.preventDefault();
      setHi((v) => (v + 1) % matches.length);
    } else if (e.key === "ArrowUp" && matches.length > 0) {
      e.preventDefault();
      setHi((v) => (v <= 0 ? matches.length - 1 : v - 1));
    } else if (e.key === "Enter") {
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
        <>
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
            role={matches.length > 0 ? "combobox" : undefined}
            aria-expanded={matches.length > 0}
            aria-autocomplete="list"
          />
          {matches.length > 0 &&
            menuRect &&
            createPortal(
              <div
                className={styles.suggestMenu}
                role="listbox"
                style={{ top: menuRect.top, left: menuRect.left }}
              >
                {matches.map((s, i) => (
                  <button
                    key={s.process}
                    type="button"
                    role="option"
                    aria-selected={i === hi}
                    className={`${styles.suggestOption} ${
                      i === hi ? styles.suggestOptionHi : ""
                    }`}
                    // mousedown + preventDefault：先于输入框 blur 落点，
                    // 否则 blur 会把半截输入原文先提交出去
                    onMouseDown={(e) => {
                      e.preventDefault();
                      commitText(s.process);
                    }}
                    onMouseEnter={() => setHi(i)}
                  >
                    <AppIcon
                      processName={s.process}
                      fallbackColor="var(--text-faint)"
                      size={14}
                    />
                    <span className={styles.suggestLabel}>{s.display}</span>
                  </button>
                ))}
              </div>,
              document.body,
            )}
        </>
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
            suggestApps
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
