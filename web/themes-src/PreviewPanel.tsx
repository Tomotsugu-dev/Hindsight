import { useEffect } from "react";
import { X } from "lucide-react";
import { STRINGS, type PageLanguage } from "./i18n";
import type { ThemeEntry } from "./index-data";
import { ThemeLinks } from "./ThemeCard";
import styles from "./ThemesPage.module.css";

interface Props {
  theme: ThemeEntry;
  language: PageLanguage;
  onClose: () => void;
}

/** 预览面板：iframe 里是套着这个主题的 demo，可以点、可以换页。 */
export function PreviewPanel({ theme, language, onClose }: Props) {
  const s = STRINGS[language];

  // Esc 关闭；打开时页面本身不跟着滚
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    document.addEventListener("keydown", onKey);
    const overflow = document.body.style.overflow;
    document.body.style.overflow = "hidden";
    return () => {
      document.removeEventListener("keydown", onKey);
      document.body.style.overflow = overflow;
    };
  }, [onClose]);

  const src = `/demo/?lang=${encodeURIComponent(language)}&theme=${encodeURIComponent(theme.id)}`;
  return (
    <div className={styles.backdrop} onClick={onClose}>
      <div
        className={styles.panel}
        role="dialog"
        aria-modal="true"
        aria-label={theme.name}
        onClick={(e) => e.stopPropagation()}
      >
        <div className={styles.panelHeader}>
          <span className={styles.panelTitle}>{theme.name}</span>
          <ThemeLinks theme={theme} language={language} />
          <button type="button" className={styles.iconButton} onClick={onClose} aria-label={s.close}>
            <X size={16} strokeWidth={2} />
          </button>
        </div>
        <iframe className={styles.frame} src={src} title={theme.name} />
      </div>
    </div>
  );
}
