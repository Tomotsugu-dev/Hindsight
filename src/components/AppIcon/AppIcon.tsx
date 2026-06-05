import { useEffect, useState } from "react";
import { api } from "../../api/hindsight";
import { lruInsert } from "../../utils/lru";
import styles from "./AppIcon.module.css";

interface CacheEntry {
  src: string | null;
  ts: number;
}

const cache = new Map<string, CacheEntry>();
const inflight = new Map<string, Promise<string | null>>();

/**
 * null 项缓存 60 秒后过期重查 —— 否则首次渲染时图标还没同步过来 cache 写了 null，
 * 之后 sync engine 拉下图标也永远不会被前端发现，必须重启 app 才能看到。
 * 命中（非 null）的项不过期：图标内容很少变，重复 invoke 浪费。
 */
const NULL_CACHE_TTL_MS = 60_000;

/**
 * base64 data URL 单个 5–30 KB（最坏 macOS .icns 512px 可达 188 KB），
 * 历史/多设备会让 unique processName 一路涨。cap 在 128 → 内存上界 ~3–24 MB；
 * LRU 命中率在常用集合（一般用户 50–100 个 app）下几乎不掉。
 */
const MAX_CACHE = 128;

function readCache(processName: string): string | null | undefined {
  const entry = cache.get(processName);
  if (!entry) return undefined;
  if (entry.src !== null) return entry.src;
  if (Date.now() - entry.ts < NULL_CACHE_TTL_MS) return null;
  // null 项过期 → 当作未缓存，触发重查
  cache.delete(processName);
  return undefined;
}

interface AppIconProps {
  processName: string;
  fallbackColor: string;
  size?: number;
}

export function AppIcon({ processName, fallbackColor, size = 18 }: AppIconProps) {
  const [src, setSrc] = useState<string | null | undefined>(() =>
    readCache(processName),
  );

  useEffect(() => {
    if (src !== undefined) return;

    const cached = readCache(processName);
    if (cached !== undefined) {
      setSrc(cached);
      return;
    }

    let cancelled = false;
    const existing = inflight.get(processName);
    const p =
      existing ??
      api
        .getAppIcon(processName)
        .then((data) => {
          lruInsert(cache, processName, { src: data ?? null, ts: Date.now() }, MAX_CACHE);
          inflight.delete(processName);
          return data ?? null;
        })
        .catch(() => {
          lruInsert(cache, processName, { src: null, ts: Date.now() }, MAX_CACHE);
          inflight.delete(processName);
          return null;
        });
    if (!existing) inflight.set(processName, p);

    void p.then((data) => {
      if (!cancelled) setSrc(data);
    });

    return () => {
      cancelled = true;
    };
  }, [processName, src]);

  if (src) {
    return (
      <img
        className={styles.icon}
        src={src}
        alt={processName}
        width={size}
        height={size}
        // <img> 默认 draggable 会拦截父元素的 HTML5 drag —— 用户从图标按下拖时
        // 浏览器触发的是 img 自身的拖图行为，不会冒泡到外层 draggable 容器。
        // 显式禁掉，让外层 chip 的 drag handler 能正常接到事件。
        draggable={false}
      />
    );
  }

  return (
    <span
      className={styles.dot}
      style={{ background: fallbackColor, width: size, height: size }}
      aria-hidden
    />
  );
}
