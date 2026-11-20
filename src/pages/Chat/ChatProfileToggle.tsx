import { useEffect, useRef, useState } from "react";
import { useNavigate } from "react-router-dom";
import { useTranslation } from "react-i18next";
import { Check, ChevronDown, ListFilter, Settings2 } from "lucide-react";
import { ROUTES } from "../../config/nav";
import type { AiProfilesState } from "../../state/aiProfiles";
import styles from "./ChatPage.module.css";

/**
 * 输入框里的「AI 能看到的数据」下拉，样子和交互跟思考档位一样：向上弹菜单。
 * 换了之后只影响后面的提问，已经回答的不变。
 */
export default function ChatProfileToggle({ state }: { state: AiProfilesState }) {
  const { t } = useTranslation();
  const navigate = useNavigate();
  const [open, setOpen] = useState(false);
  const wrapRef = useRef<HTMLSpanElement>(null);

  useEffect(() => {
    if (!open) return;
    const onDown = (e: MouseEvent) => {
      if (wrapRef.current && !wrapRef.current.contains(e.target as Node)) {
        setOpen(false);
      }
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") setOpen(false);
    };
    document.addEventListener("mousedown", onDown);
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("mousedown", onDown);
      document.removeEventListener("keydown", onKey);
    };
  }, [open]);

  const items: [string | null, string][] = [
    [null, t("aiProfiles.main")],
    ...state.list.map(([id, p]): [string, string] => [id, p.name]),
  ];
  const currentLabel = items.find(([id]) => id === state.selected)?.[1] ?? t("aiProfiles.main");

  const pick = (id: string | null) => {
    setOpen(false);
    state.select(id);
  };

  return (
    <span ref={wrapRef} className={styles.thinkWrap}>
      <button
        type="button"
        className={styles.thinkBtn}
        title={t("aiProfiles.pickerTitle")}
        onClick={() => setOpen((v) => !v)}
        aria-haspopup="menu"
        aria-expanded={open}
        aria-label={`${t("aiProfiles.pickerTitle")}: ${currentLabel}`}
      >
        <ListFilter size={13} strokeWidth={2} aria-hidden />
        <span className={styles.profileLabel}>{currentLabel}</span>
        <ChevronDown size={12} strokeWidth={2.2} />
      </button>

      {open && (
        <div className={`${styles.badgeMenu} ${styles.thinkMenu}`} role="menu">
          {items.map(([id, label]) => (
            <button
              key={id ?? "main"}
              type="button"
              role="menuitem"
              className={styles.badgeMenuItem}
              onClick={() => pick(id)}
            >
              <span className={styles.badgeMenuLabel}>{label}</span>
              {state.selected === id && <Check size={12} strokeWidth={2.4} />}
            </button>
          ))}
          <div className={styles.menuDivider} aria-hidden />
          <button
            type="button"
            role="menuitem"
            className={`${styles.badgeMenuItem} ${styles.menuAction}`}
            onClick={() => navigate(`${ROUTES.aiSettings}/general?module=chat`)}
          >
            <Settings2 size={13} strokeWidth={2} aria-hidden />
            <span className={styles.badgeMenuLabel}>{t("aiProfiles.manage")}</span>
          </button>
        </div>
      )}
    </span>
  );
}
