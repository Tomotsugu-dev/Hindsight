import { useEffect, useRef, useState } from "react";
import { convertFileSrc } from "@tauri-apps/api/core";
import { api } from "../../api/hindsight";
import { lruInsert } from "../../utils/lru";
import styles from "./AppIcon.module.css";

interface CacheEntry {
  /** 后端返回的图标文件**绝对路径**；为 null 表示曾经查过、当时没有图标。 */
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
 * cache 现在存的是几十字节的文件路径（不再是几十~两百 KB 的 base64 data URL），
 * 实际尺寸对内存影响很小；LRU 保留主要是 invoke 去重 + 防止 unique processName
 * 失控（极端用户跑过 N 千个 app）。128 足够覆盖常用集合。
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

  // processName 变化时重置 src（React 官方"prop 变化调整 state"的渲染期模式）：
  // 不重置的话下面 effect 因 src !== undefined 直接 return，被复用的实例
  //（合并/拆分应用组后同一行换了 iconProcess）会永远显示旧应用的图标。
  const prevNameRef = useRef(processName);
  if (prevNameRef.current !== processName) {
    prevNameRef.current = processName;
    setSrc(readCache(processName));
  }

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
        // 后端返绝对路径，convertFileSrc 转 asset://localhost/<url-encoded-path>
        // 让 WKWebView 自己缓存/淘汰图像数据（不再压在 JS heap 上）
        src={convertFileSrc(src)}
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
