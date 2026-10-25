import type { CSSProperties } from "react";
import { CloudDownload, Download, ExternalLink, Eye } from "lucide-react";
import { pickDescription } from "@app/utils/pickDescription";
import { STRINGS, type PageLanguage } from "./i18n";
import type { ThemeEntry } from "./index-data";
import { swatchColors } from "./swatch";
import styles from "./ThemesPage.module.css";

interface Props {
  theme: ThemeEntry;
  language: PageLanguage;
  onPreview: (theme: ThemeEntry) => void;
}

/** 一个主题的卡片：色块、名称、作者和版本、简介、下载次数，以及三个按钮。 */
export function ThemeCard({ theme, language, onPreview }: Props) {
  const s = STRINGS[language];
  const description = pickDescription(theme.description, language);
  const downloads = new Intl.NumberFormat(language).format(theme.downloads);
  return (
    <article className={styles.card}>
      <Swatch theme={theme} />
      <div className={styles.cardBody}>
        <h2 className={styles.name}>{theme.name}</h2>
        <p className={styles.meta}>{[theme.author, theme.version].filter(Boolean).join(" · ")}</p>
        {description && <p className={styles.description}>{description}</p>}
        <p className={styles.downloads}>
          <CloudDownload size={14} strokeWidth={2} aria-hidden />
          {s.downloads(downloads)}
        </p>
      </div>
      <div className={styles.actions}>
        <button type="button" className={`${styles.button} ${styles.primary}`} onClick={() => onPreview(theme)}>
          <Eye size={14} strokeWidth={2} aria-hidden />
          {s.preview}
        </button>
        <ThemeLinks theme={theme} language={language} />
      </div>
    </article>
  );
}

/** 「主页」和「下载」：卡片上和预览面板里都有。 */
export function ThemeLinks({ theme, language }: { theme: ThemeEntry; language: PageLanguage }) {
  const s = STRINGS[language];
  return (
    <>
      <a className={styles.button} href={theme.homepage} target="_blank" rel="noopener noreferrer">
        <ExternalLink size={14} strokeWidth={2} aria-hidden />
        {s.homepage}
      </a>
      <a className={styles.button} href={theme.release_url} target="_blank" rel="noopener noreferrer">
        <Download size={14} strokeWidth={2} aria-hidden />
        {s.download}
      </a>
    </>
  );
}

// 小图里柱子的高度（%），只是装饰
const BARS = [45, 70, 55, 90, 65, 80, 50];

/** 用主题的颜色画一个缩小的窗口：侧栏、页面底色、卡片、强调色和文字。 */
function Swatch({ theme }: { theme: ThemeEntry }) {
  const c = swatchColors(theme.mode, theme.tokens);
  const vars = {
    "--sw-sidebar": c["--sidebar-gradient"],
    "--sw-app": c["--surface-app"],
    "--sw-card": c["--surface-card"],
    "--sw-border": c["--border-subtle"],
    "--sw-accent": c["--color-accent"],
    "--sw-strong": c["--text-strong"],
    "--sw-muted": c["--text-muted"],
  } as CSSProperties;
  return (
    <div className={styles.swatch} style={vars} aria-hidden>
      <div className={styles.swatchSidebar}>
        <span className={`${styles.swatchNav} ${styles.swatchNavActive}`} />
        <span className={styles.swatchNav} />
        <span className={styles.swatchNav} />
        <span className={styles.swatchNav} />
      </div>
      <div className={styles.swatchMain}>
        <div className={styles.swatchCard}>
          <span className={styles.swatchTitle} />
          <span className={styles.swatchLine} />
          <div className={styles.swatchBars}>
            {BARS.map((height, i) => (
              <span key={i} className={styles.swatchBar} style={{ height: `${height}%` }} />
            ))}
          </div>
        </div>
      </div>
    </div>
  );
}
