import { useCallback, useEffect, useState } from "react";
import { RotateCw } from "lucide-react";
import { DOCS_PAGE, HOME_PAGE, LANGUAGE_NAMES, STRINGS, pageLanguage, type PageLanguage } from "./i18n";
import { loadIndex, type ThemeEntry } from "./index-data";
import { PreviewPanel } from "./PreviewPanel";
import { ThemeCard } from "./ThemeCard";
import styles from "./ThemesPage.module.css";

const LANGUAGE = pageLanguage(location.search, navigator.languages);
// site.css 按 <html lang> 选字体，渲染前就设好
document.documentElement.lang = LANGUAGE;

const LOGO = "https://cdn.jsdelivr.net/gh/Tomotsugu-dev/Hindsight@main/src/assets/logo.png";

// 怎么上架主题。那一节开头再链到主题文件的格式
const MAKE_A_THEME =
  "https://github.com/Tomotsugu-dev/hindsight-plugins/blob/main/CONTRIBUTING.md#themes";

type State =
  | { status: "loading" }
  | { status: "failed"; reason: string }
  | { status: "ready"; themes: ThemeEntry[] };

/** 官网的 /themes/：主题列表，可以预览、去作者主页、下载。 */
export function ThemesPage() {
  const s = STRINGS[LANGUAGE];
  const [state, setState] = useState<State>({ status: "loading" });
  const [previewing, setPreviewing] = useState<ThemeEntry | null>(null);

  const load = useCallback(() => {
    setState({ status: "loading" });
    loadIndex().then(
      (themes) => setState({ status: "ready", themes }),
      (e: unknown) => setState({ status: "failed", reason: e instanceof Error ? e.message : String(e) }),
    );
  }, []);

  useEffect(() => {
    document.title = `${s.title} · Hindsight`;
    load();
  }, [load, s.title]);

  return (
    <>
      <TopLine />

      <main className={`page ${styles.main}`}>
        <header className={styles.header}>
          <h1 className={styles.title}>{s.title}</h1>
          <p className={styles.intro}>{s.intro}</p>
          <p className={styles.install}>{s.install}</p>
        </header>

        {state.status === "loading" && <p className={styles.state}>{s.loading}</p>}

        {state.status === "failed" && (
          <div className={styles.state}>
            <p>{s.loadFailed(state.reason)}</p>
            <button type="button" className={styles.button} onClick={load}>
              <RotateCw size={14} strokeWidth={2} aria-hidden />
              {s.retry}
            </button>
          </div>
        )}

        {state.status === "ready" && state.themes.length === 0 && (
          <div className={styles.state}>
            <p>{s.empty}</p>
            <a className={styles.button} href={MAKE_A_THEME} target="_blank" rel="noopener noreferrer">
              {s.makeOne}
            </a>
          </div>
        )}

        {state.status === "ready" && state.themes.length > 0 && (
          <div className={styles.grid}>
            {state.themes.map((theme) => (
              <ThemeCard key={theme.id} theme={theme} language={LANGUAGE} onPreview={setPreviewing} />
            ))}
          </div>
        )}
      </main>

      <Footer />

      {previewing && (
        <PreviewPanel theme={previewing} language={LANGUAGE} onClose={() => setPreviewing(null)} />
      )}
    </>
  );
}

/** 和官网首页一样的顶栏。页面导航链到这种语言首页的各节。 */
function TopLine() {
  const s = STRINGS[LANGUAGE];
  const home = HOME_PAGE[LANGUAGE];
  return (
    <header className="topline">
      <div className="page topline-inner">
        <a href={home} className="brand">
          <img src={LOGO} alt="" width={28} height={28} />
          <span>Hindsight</span>
        </a>
        <nav className="topline-links" aria-label={s.navLabel}>
          <a href={`${home}#features`}>{s.navFeatures}</a>
          <a href={`${home}#demo`}>{s.navDemo}</a>
          <a href={`${home}#download`}>{s.navDownload}</a>
        </nav>
        <LanguageSwitch />
        <a href="https://github.com/Tomotsugu-dev/Hindsight" className="topline-github" aria-label="GitHub">
          <GitHubIcon />
        </a>
      </div>
    </header>
  );
}

/** 和官网首页一样的语言切换。换语言就是带着 ?lang= 重新打开这一页。 */
function LanguageSwitch() {
  const s = STRINGS[LANGUAGE];
  const languages = Object.keys(LANGUAGE_NAMES) as PageLanguage[];
  return (
    <details className="lang">
      <summary aria-label={s.language}>
        <GlobeIcon />
        {LANGUAGE_NAMES[LANGUAGE]}
      </summary>
      <ul>
        {languages.map((code) => (
          <li key={code}>
            <a href={`?lang=${code}`} lang={code} aria-current={code === LANGUAGE ? "page" : undefined}>
              {LANGUAGE_NAMES[code]}
            </a>
          </li>
        ))}
      </ul>
    </details>
  );
}

/** 和官网首页一样的页脚。 */
function Footer() {
  const s = STRINGS[LANGUAGE];
  return (
    <footer className="foot">
      <div className="page foot-grid">
        <div className="foot-brand">
          <p className="foot-name">Hindsight</p>
          <p>© 2026 Hindsight contributors · GPL 3.0</p>
        </div>
        <ul>
          <li>
            <a href="https://github.com/Tomotsugu-dev/Hindsight/releases">{s.allReleases}</a>
          </li>
          <li>
            <a href="https://github.com/Tomotsugu-dev/Hindsight/tree/main/docs/release-notes">{s.changelog}</a>
          </li>
          <li>
            <a href={DOCS_PAGE[LANGUAGE]}>{s.docs}</a>
          </li>
        </ul>
        <ul>
          <li>
            <a href="https://github.com/Tomotsugu-dev/Hindsight">{s.repository}</a>
          </li>
          <li>
            <a href="https://github.com/Tomotsugu-dev/Hindsight/issues">{s.reportProblem}</a>
          </li>
          <li>
            <a href={`?lang=${LANGUAGE}`} aria-current="page">
              {s.themeMarket}
            </a>
          </li>
        </ul>
      </div>
    </footer>
  );
}

/** 首页顶栏用的地球图标，和 lucide 的画法不一样 */
function GlobeIcon() {
  return (
    <svg
      width="16"
      height="16"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="2"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden
    >
      <circle cx="12" cy="12" r="10" />
      <path d="M12 2a15.3 15.3 0 0 1 4 10 15.3 15.3 0 0 1-4 10 15.3 15.3 0 0 1-4-10 15.3 15.3 0 0 1 4-10z" />
      <line x1="2" y1="12" x2="22" y2="12" />
    </svg>
  );
}

function GitHubIcon() {
  return (
    <svg width="20" height="20" viewBox="0 0 24 24" fill="currentColor" aria-hidden>
      <path d="M12 .5C5.65.5.5 5.65.5 12c0 5.08 3.29 9.39 7.86 10.92.58.1.79-.25.79-.56 0-.28-.01-1.02-.02-2-3.2.7-3.87-1.54-3.87-1.54-.52-1.33-1.28-1.69-1.28-1.69-1.05-.72.08-.7.08-.7 1.16.08 1.77 1.19 1.77 1.19 1.03 1.77 2.7 1.26 3.36.96.1-.75.4-1.26.73-1.55-2.55-.29-5.23-1.28-5.23-5.69 0-1.26.45-2.29 1.19-3.1-.12-.29-.52-1.47.11-3.05 0 0 .97-.31 3.18 1.18a11.05 11.05 0 0 1 5.79 0c2.2-1.49 3.17-1.18 3.17-1.18.63 1.58.23 2.76.12 3.05.74.81 1.18 1.84 1.18 3.1 0 4.42-2.69 5.4-5.25 5.68.41.36.78 1.06.78 2.14 0 1.55-.01 2.79-.01 3.17 0 .31.21.67.8.55C20.21 21.39 23.5 17.08 23.5 12 23.5 5.65 18.35.5 12 .5z" />
    </svg>
  );
}
