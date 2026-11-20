import { useCallback, useEffect, useRef, useState } from "react";
import { X } from "lucide-react";
import type { CustomTheme } from "@app/lib/theme";
import { STRINGS, type PageLanguage } from "./i18n";
import type { ThemeEntry } from "./index-data";
import { getCachedTheme, loadFullTheme } from "./theme-cache";
import { ThemeLinks } from "./ThemeCard";
import styles from "./ThemesPage.module.css";

// 与 web/demo-src/preview-theme.ts 约定的同源 postMessage 协议
const PREVIEW_THEME_MSG = "hindsight:preview-theme";
const PREVIEW_THEME_ERROR_MSG = "hindsight:preview-theme-error";
const PREVIEW_HOST_READY_MSG = "hindsight:preview-host-ready";

type HostMessage =
  | { type: typeof PREVIEW_THEME_MSG; theme: CustomTheme; resetRoute?: boolean }
  | { type: typeof PREVIEW_THEME_ERROR_MSG };

interface Props {
  /** 当前正在预览的主题；为 null 时弹窗隐藏，但常驻 iframe 保持挂载不销毁，下次打开零启动开销。 */
  theme: ThemeEntry | null;
  language: PageLanguage;
  onClose: () => void;
}

/** 预览面板：常驻复用的 demo iframe，通过同源 postMessage 瞬时切换主题，可以点、可以换页。 */
export function PreviewPanel({ theme, language, onClose }: Props) {
  const s = STRINGS[language];
  const open = theme !== null;
  const iframeRef = useRef<HTMLIFrameElement | null>(null);
  const readyRef = useRef(false);
  const pendingMsgRef = useRef<HostMessage | null>(null);
  const [hostReady, setHostReady] = useState(false);

  const postToHost = useCallback((msg: HostMessage) => {
    pendingMsgRef.current = msg;
    const target = iframeRef.current?.contentWindow;
    if (readyRef.current && target) {
      target.postMessage(msg, window.location.origin);
    }
  }, []);

  // 监听 demo 宿主 iframe 发出的就绪信号；就绪后把待发主题立刻推送进去
  useEffect(() => {
    const onMessage = (event: MessageEvent) => {
      if (event.origin !== window.location.origin) return;
      if (event.source !== iframeRef.current?.contentWindow) return;
      const data = event.data as { type?: unknown } | null;
      if (data?.type !== PREVIEW_HOST_READY_MSG) return;

      readyRef.current = true;
      setHostReady(true);
      if (pendingMsgRef.current && iframeRef.current?.contentWindow) {
        iframeRef.current.contentWindow.postMessage(pendingMsgRef.current, window.location.origin);
      }
    };
    window.addEventListener("message", onMessage);
    return () => window.removeEventListener("message", onMessage);
  }, []);

  // 打开预览时：Esc 关闭、锁住外层滚动
  useEffect(() => {
    if (!open) return;
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
  }, [open, onClose]);

  // 切到某个主题时：命中内存缓存直接推完整版；否则先推 index.json 的 tokens 瞬时上色，
  // 再异步拉取完整 <id>.json 补齐 theme.css（防快速连点竞态）
  useEffect(() => {
    if (!theme) return;
    let cancelled = false;

    const cached = getCachedTheme(theme.id);
    if (cached) {
      postToHost({ type: PREVIEW_THEME_MSG, theme: cached, resetRoute: true });
      return;
    }

    // 立即套用 index.json 自带的 tokens / layout / mode，0ms 消除白屏和旧主题残留
    postToHost({
      type: PREVIEW_THEME_MSG,
      theme: {
        id: theme.id,
        layout: theme.layout,
        mode: theme.mode,
        tokens: theme.tokens,
        css: null,
      },
      resetRoute: true,
    });

    loadFullTheme(theme.id).then(
      (full) => {
        // 仅当包含额外 theme.css（或与基础快照存在差异）时再补推，不重置用户已切换的页面
        if (!cancelled && full.css !== null) {
          postToHost({ type: PREVIEW_THEME_MSG, theme: full, resetRoute: false });
        }
      },
      (e: unknown) => {
        console.error("[themes] preview theme", e);
        if (!cancelled) postToHost({ type: PREVIEW_THEME_ERROR_MSG });
      },
    );

    return () => {
      cancelled = true;
    };
  }, [theme, postToHost]);

  const src = `/demo/?lang=${encodeURIComponent(language)}&theme-host=1`;
  return (
    <div
      className={`${styles.backdrop} ${open ? "" : styles.backdropHidden}`}
      aria-hidden={!open || undefined}
      onClick={open ? onClose : undefined}
    >
      <div
        className={styles.panel}
        role="dialog"
        aria-modal={open || undefined}
        aria-label={theme?.name ?? s.preview}
        onClick={(e) => e.stopPropagation()}
      >
        <div className={styles.panelHeader}>
          <span className={styles.panelTitle}>{theme?.name ?? ""}</span>
          {theme && <ThemeLinks theme={theme} language={language} />}
          <button
            type="button"
            className={styles.iconButton}
            onClick={onClose}
            tabIndex={open ? 0 : -1}
            aria-label={s.close}
          >
            <X size={16} strokeWidth={2} />
          </button>
        </div>
        <iframe
          ref={iframeRef}
          className={`${styles.frame} ${hostReady ? "" : styles.frameLoading}`}
          src={src}
          title={theme?.name ?? s.preview}
          tabIndex={open ? 0 : -1}
        />
      </div>
    </div>
  );
}
