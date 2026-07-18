import { useEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { useTranslation } from "react-i18next";
import { convertFileSrc } from "@tauri-apps/api/core";
import { ImageOff, Loader2, ScanSearch, Search } from "lucide-react";
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
      setHits((prev) => [...prev, ...resp.hits]);
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
      <h1 className={styles.title}>{t("search.pageTitle")}</h1>
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

      {viewer &&
        createPortal(
          <div
            className={styles.viewerBackdrop}
            onMouseDown={() => setViewer(null)}
            role="presentation"
          >
            <div className={styles.viewerBody}>
              {viewer.framePath ? (
                <img
                  className={styles.viewerImg}
                  src={convertFileSrc(viewer.framePath)}
                  alt={t("search.viewerAlt")}
                />
              ) : (
                <div className={styles.viewerMissing}>
                  <ImageOff size={32} strokeWidth={1.5} />
                  {t("search.imageGone")}
                </div>
              )}
              <p className={styles.viewerMeta}>
                {viewer.app}
                {viewer.title ? ` · ${viewer.title}` : ""}
                {" · "}
                {fmtTs(viewer.frameTs ?? viewer.startedTs)}
              </p>
            </div>
          </div>,
          document.body,
        )}
    </div>
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
