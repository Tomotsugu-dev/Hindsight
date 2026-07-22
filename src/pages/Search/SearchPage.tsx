import { useEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { useTranslation } from "react-i18next";
import { convertFileSrc } from "@tauri-apps/api/core";
import { ImageOff, Loader2, ScanSearch, Search, ZoomIn, ZoomOut } from "lucide-react";
import { api, type MemorySearchHit } from "../../api/hindsight";
import { EmptyHint } from "../../components/EmptyHint/EmptyHint";
import { logError } from "../../lib/logger";
import styles from "./SearchPage.module.css";

/** 每页条数；"加载更多"按此步长追加。 */
const PAGE_SIZE = 30;
/** 输入防抖:本地 FTS 毫秒级,300ms 足够把连续击键合并成一次查询。 */
const DEBOUNCE_MS = 300;

/** RFC3339 → "MM-DD HH:mm"(本地时区)。 */
function fmtTs(ts: string): string {
  const d = new Date(ts);
  if (Number.isNaN(d.getTime())) return ts;
  const p = (n: number): string => String(n).padStart(2, "0");
  return `${p(d.getMonth() + 1)}-${p(d.getDate())} ${p(d.getHours())}:${p(d.getMinutes())}`;
}

/** 只取 "HH:mm"(同日范围的结束端不重复日期)。 */
function fmtHm(ts: string): string {
  const d = new Date(ts);
  if (Number.isNaN(d.getTime())) return ts;
  const p = (n: number): string => String(n).padStart(2, "0");
  return `${p(d.getHours())}:${p(d.getMinutes())}`;
}

/** snippet 里把命中词标成 <mark>。多词 alternation 一次 split,奇数段即命中。 */
function Highlight({ text, words }: { text: string; words: string[] }) {
  const escaped = words
    .filter((w) => w.length > 0)
    .map((w) => w.replace(/[.*+?^${}()|[\]\\]/g, "\\$&"));
  if (escaped.length === 0) return <>{text}</>;
  const re = new RegExp(`(${escaped.join("|")})`, "gi");
  const parts = text.split(re);
  return (
    <>
      {parts.map((part, i) =>
        i % 2 === 1 ? (
          <mark key={i} className={styles.mark}>
            {part}
          </mark>
        ) : (
          <span key={i}>{part}</span>
        ),
      )}
    </>
  );
}

/**
 * 屏幕记忆搜索页:搜索截图 OCR 后的屏幕文字,命中定位到会话时间与具体截图。
 * 数据链路:text_sessions_fts(trigram)→ 会话时间范围 → session_lines 首现帧。
 * 截图受保留策略约束,文件可能已被清理——缩略图/大图都做缺图兜底。
 */
export default function SearchPage() {
  const { t } = useTranslation();
  const [query, setQuery] = useState("");
  const [hits, setHits] = useState<MemorySearchHit[]>([]);
  const [total, setTotal] = useState(0);
  const [searching, setSearching] = useState(false);
  const [loadingMore, setLoadingMore] = useState(false);
  const [error, setError] = useState<string | null>(null);
  /** 大图预览的当前命中;null = 关闭 */
  const [viewer, setViewer] = useState<MemorySearchHit | null>(null);
  /** 竞态防护:只采纳最后一次发出的查询的结果 */
  const seqRef = useRef(0);

  const words = query.trim().split(/\s+/).filter(Boolean);

  // 输入防抖自动搜;query 清空立即复位
  useEffect(() => {
    const trimmed = query.trim();
    const seq = ++seqRef.current;
    if (trimmed.length === 0) {
      setHits([]);
      setTotal(0);
      setSearching(false);
      setError(null);
      return;
    }
    setSearching(true);
    const timer = setTimeout(() => {
      api
        .memorySearch(trimmed, PAGE_SIZE, 0)
        .then((resp) => {
          if (seqRef.current !== seq) return; // 已有更新的查询在途
          setHits(resp.hits);
          setTotal(resp.total);
          setError(null);
        })
        .catch((e) => {
          if (seqRef.current !== seq) return;
          logError("search.query", e);
          setHits([]);
          setTotal(0);
          setError(typeof e === "string" ? e : String(e));
        })
        .finally(() => {
          if (seqRef.current === seq) setSearching(false);
        });
    }, DEBOUNCE_MS);
    return () => clearTimeout(timer);
  }, [query]);

  const loadMore = async () => {
    const seq = seqRef.current; // 不递增:追加页属于当前查询
    setLoadingMore(true);
    try {
      const resp = await api.memorySearch(query.trim(), PAGE_SIZE, hits.length);
      if (seqRef.current !== seq) return;
      // 常驻 OCR 在持续插入新会话,offset 窗口会漂移——按 sessionId 去重,
      // 防止"加载更多"重复条目(也避免 React key 冲突)
      setHits((prev) => {
        const seen = new Set(prev.map((h) => h.sessionId));
        return [...prev, ...resp.hits.filter((h) => !seen.has(h.sessionId))];
      });
      setTotal(resp.total);
    } catch (e) {
      if (seqRef.current === seq) logError("search.loadMore", e);
    } finally {
      if (seqRef.current === seq) setLoadingMore(false);
    }
  };

  // Esc 关闭大图
  useEffect(() => {
    if (!viewer) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") setViewer(null);
    };
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, [viewer]);

  const showInitial = query.trim().length === 0;
  const showEmpty = !showInitial && !searching && !error && hits.length === 0;

  return (
    <div className={styles.page}>
      <p className={styles.subtitle}>{t("search.subtitle")}</p>

      <div className={styles.searchBox}>
        <Search size={15} strokeWidth={2} className={styles.searchIcon} />
        <input
          type="text"
          className={styles.searchInput}
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          placeholder={t("search.placeholder")}
          autoComplete="off"
          autoCorrect="off"
          spellCheck={false}
          // eslint-disable-next-line jsx-a11y/no-autofocus
          autoFocus
        />
        {searching && <Loader2 size={14} strokeWidth={2.25} className={styles.searchSpin} />}
      </div>

      {!showInitial && !searching && !error && hits.length > 0 && (
        <p className={styles.totalLine}>{t("search.total", { count: total })}</p>
      )}
      {error && <p className={styles.errorLine}>{t("search.unavailable", { message: error })}</p>}

      {showInitial && (
        <div className={styles.initial}>
          <ScanSearch size={36} strokeWidth={1.4} className={styles.initialIcon} />
          <p className={styles.initialText}>{t("search.initial")}</p>
          <p className={styles.initialHint}>{t("search.hint")}</p>
        </div>
      )}

      {showEmpty && <EmptyHint message={t("search.empty")} />}
      {showEmpty && <p className={styles.emptyHint}>{t("search.emptyHint")}</p>}

      <div className={styles.hitList}>
        {hits.map((h) => (
          <HitCard key={h.sessionId} hit={h} words={words} onOpen={() => setViewer(h)} />
        ))}
      </div>

      {hits.length < total && (
        <button
          type="button"
          className={styles.loadMoreBtn}
          onClick={() => void loadMore()}
          disabled={loadingMore}
        >
          {loadingMore ? (
            <Loader2 size={13} strokeWidth={2.25} className={styles.searchSpin} />
          ) : null}
          {t("search.loadMore", { count: total - hits.length })}
        </button>
      )}

      {viewer && <Viewer hit={viewer} words={words} onClose={() => setViewer(null)} />}
    </div>
  );
}

/** 缩放范围与步进(滚轮/按钮共用)。 */
const ZOOM_MIN = 1;
const ZOOM_MAX = 8;
const ZOOM_STEP = 1.25;

/**
 * 截图大图预览:滚轮/按钮缩放、拖拽平移、双击复位;
 * 打开时现场 OCR 该帧定位命中行,画半透明高亮框(历史帧同样可用;
 * OCR 失败或图已清理则只展示图/占位,优雅降级)。
 */
function Viewer({
  hit,
  words,
  onClose,
}: {
  hit: MemorySearchHit;
  words: string[];
  onClose: () => void;
}) {
  const { t } = useTranslation();
  const [zoom, setZoom] = useState(1);
  const [pan, setPan] = useState({ x: 0, y: 0 });
  const [boxes, setBoxes] = useState<[number, number, number, number][]>([]);
  const [locating, setLocating] = useState(false);
  const [dragging, setDragging] = useState(false);
  /** 大图加载失败(文件已被保留策略清理)→ 降级文字视图 */
  const [imgGone, setImgGone] = useState(false);
  const [sessionText, setSessionText] = useState<string | null>(null);
  const dragRef = useRef<{ x: number; y: number } | null>(null);
  const showText = !hit.framePath || imgGone;

  // 图不可用 → 拉会话 OCR 全文(图没了字还在)
  useEffect(() => {
    if (!showText || sessionText !== null) return;
    let alive = true;
    api
      .memorySessionText(hit.sessionId)
      .then((text) => {
        if (alive) setSessionText(text);
      })
      .catch((e) => logError("search.sessionText", e));
    return () => {
      alive = false;
    };
  }, [showText, sessionText, hit.sessionId]);

  // 打开即定位命中行;组件卸载(关闭)后丢弃结果
  useEffect(() => {
    if (!hit.framePath) return;
    let alive = true;
    setLocating(true);
    api
      .memoryLocate(hit.framePath, words)
      .then((b) => {
        if (alive) setBoxes(b);
      })
      .catch((e) => logError("search.locate", e))
      .finally(() => {
        if (alive) setLocating(false);
      });
    return () => {
      alive = false;
    };
    // words 随输入框变化,但 viewer 打开期间应按打开时的词定位——只跟 framePath
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [hit.framePath]);

  const clampZoom = (z: number) => Math.min(ZOOM_MAX, Math.max(ZOOM_MIN, z));
  const applyZoom = (factor: number) => {
    setZoom((z) => {
      const next = clampZoom(z * factor);
      if (next === ZOOM_MIN) setPan({ x: 0, y: 0 });
      return next;
    });
  };
  const reset = () => {
    setZoom(1);
    setPan({ x: 0, y: 0 });
  };

  return createPortal(
    <div className={styles.viewerBackdrop} onMouseDown={onClose} role="presentation">
      <div
        className={styles.viewerBody}
        role="presentation"
        onMouseDown={(e) => e.stopPropagation()}
        onWheel={(e) => applyZoom(e.deltaY < 0 ? ZOOM_STEP : 1 / ZOOM_STEP)}
      >
        {!showText ? (
          <div
            className={`${styles.viewerCanvas} ${zoom > 1 ? styles.viewerCanvasPannable : ""}`}
            role="presentation"
            onPointerDown={(e) => {
              if (zoom <= 1) return;
              e.preventDefault();
              // 指针捕获:光标拖出画布甚至窗口,手势也不断
              e.currentTarget.setPointerCapture(e.pointerId);
              dragRef.current = { x: e.clientX - pan.x, y: e.clientY - pan.y };
              setDragging(true);
            }}
            onPointerMove={(e) => {
              if (!dragRef.current) return;
              setPan({ x: e.clientX - dragRef.current.x, y: e.clientY - dragRef.current.y });
            }}
            onPointerUp={(e) => {
              dragRef.current = null;
              setDragging(false);
              e.currentTarget.releasePointerCapture(e.pointerId);
            }}
            onPointerCancel={() => {
              dragRef.current = null;
              setDragging(false);
            }}
            onDoubleClick={reset}
          >
            <div
              className={styles.viewerTransform}
              style={
                {
                  transform: `translate(${pan.x}px, ${pan.y}px) scale(${zoom})`,
                  transition: dragging ? "none" : undefined,
                  // 高亮框线宽用它做反向补偿:图放大 8x 时线仍是视觉 1.5px
                  "--viewer-zoom": zoom,
                } as React.CSSProperties
              }
            >
              {/* onError 是缺图降级(文件被保留策略清理),不是交互事件 */}
              {/* eslint-disable-next-line jsx-a11y/no-noninteractive-element-interactions */}
              <img
                className={styles.viewerImg}
                src={convertFileSrc(hit.framePath!)}
                alt={t("search.viewerAlt")}
                draggable={false}
                onError={() => setImgGone(true)}
              />
              {boxes.map((b, i) => (
                <div
                  key={i}
                  className={styles.viewerMark}
                  style={{
                    left: `${b[0] * 100}%`,
                    top: `${b[1] * 100}%`,
                    width: `${b[2] * 100}%`,
                    height: `${b[3] * 100}%`,
                  }}
                />
              ))}
            </div>
          </div>
        ) : (
          <div className={styles.viewerText}>
            <p className={styles.viewerTextNotice}>
              <ImageOff size={13} strokeWidth={1.8} />
              {t("search.textFallbackNotice")}
            </p>
            <div className={styles.viewerTextBody}>
              {sessionText === null ? (
                <Loader2 size={16} strokeWidth={2} className={styles.searchSpin} />
              ) : (
                <Highlight text={sessionText} words={words} />
              )}
            </div>
          </div>
        )}
        <div className={styles.viewerBar}>
          <p className={styles.viewerMeta}>
            {hit.app}
            {hit.title ? ` · ${hit.title}` : ""}
            {" · "}
            {fmtTs(hit.frameTs ?? hit.startedTs)}
            {locating ? ` · ${t("search.locating")}` : ""}
          </p>
          {!showText ? (
            <div className={styles.viewerZoomCtl}>
              <button
                type="button"
                className={styles.viewerZoomBtn}
                onClick={() => applyZoom(1 / ZOOM_STEP)}
                aria-label={t("search.zoomOut")}
              >
                <ZoomOut size={14} strokeWidth={2} />
              </button>
              <button
                type="button"
                className={styles.viewerZoomBtn}
                onClick={reset}
                aria-label={t("search.zoomReset")}
              >
                {Math.round(zoom * 100)}%
              </button>
              <button
                type="button"
                className={styles.viewerZoomBtn}
                onClick={() => applyZoom(ZOOM_STEP)}
                aria-label={t("search.zoomIn")}
              >
                <ZoomIn size={14} strokeWidth={2} />
              </button>
            </div>
          ) : null}
        </div>
      </div>
    </div>,
    document.body,
  );
}

/** 一条命中:缩略图 + 时间/应用/命中片段。缩略图加载失败(截图已清理)显示占位。 */
function HitCard({
  hit,
  words,
  onOpen,
}: {
  hit: MemorySearchHit;
  words: string[];
  onOpen: () => void;
}) {
  const { t } = useTranslation();
  const [imgFailed, setImgFailed] = useState(false);
  const hasImage = hit.framePath !== null && !imgFailed;

  return (
    <button type="button" className={styles.hit} onClick={onOpen}>
      <div className={styles.hitThumbWrap}>
        {hasImage ? (
          // onError 是加载失败兜底(截图已被保留策略清理),不是交互事件
          // eslint-disable-next-line jsx-a11y/no-noninteractive-element-interactions
          <img
            className={styles.hitThumb}
            src={convertFileSrc(hit.framePath!)}
            alt=""
            loading="lazy"
            onError={() => setImgFailed(true)}
          />
        ) : (
          <div className={styles.hitThumbMissing} title={t("search.imageGone")}>
            <ImageOff size={18} strokeWidth={1.6} />
          </div>
        )}
      </div>
      <div className={styles.hitBody}>
        <div className={styles.hitHead}>
          <span className={styles.hitApp}>
            {hit.app}
            {hit.title ? ` · ${hit.title}` : ""}
          </span>
          <span className={styles.hitTime}>
            {fmtTs(hit.startedTs)} – {fmtHm(hit.endedTs)}
          </span>
        </div>
        <p className={styles.hitSnippet}>
          <Highlight text={hit.snippet} words={words} />
        </p>
      </div>
    </button>
  );
}
