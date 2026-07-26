import { useEffect, useRef, useState, type RefObject } from "react";
import styles from "./ScrollIndicator.module.css";
import { thumbGeometry, type ThumbGeometry } from "./thumbGeometry";

interface ScrollIndicatorProps {
  /** 被指示的滚动容器。组件渲染为它的兄弟,轨道位置由宿主 className 定。 */
  targetRef: RefObject<HTMLElement | null>;
  /** 轨道定位类(top/bottom/right/width/z-index 全由宿主 CSS 决定) */
  className?: string;
}

/**
 * 自绘滚动条:常驻可见的细 thumb + 可拖拽。
 *
 * 存在的理由见 [`thumbGeometry`] 的模块注释(WKWebView 画不出自定义原生条)。
 * 用法:宿主把滚动容器的原生滚动条藏掉(scrollbar-width: none +
 * ::-webkit-scrollbar display:none),在容器**外面**挂本组件并用 className
 * 把轨道定位到容器右缘——放容器里面会跟着内容一起滚走。
 */
export function ScrollIndicator({ targetRef, className }: ScrollIndicatorProps) {
  const trackRef = useRef<HTMLDivElement>(null);
  const [geo, setGeo] = useState<ThumbGeometry | null>(null);
  const [dragging, setDragging] = useState(false);
  const [scrolling, setScrolling] = useState(false);
  const scrollTimer = useRef<number | null>(null);
  const drag = useRef<{ startY: number; startScrollTop: number } | null>(null);

  useEffect(() => {
    const el = targetRef.current;
    if (!el) return;

    const sync = () =>
      setGeo(thumbGeometry(el.scrollTop, el.scrollHeight, el.clientHeight));
    const onScroll = () => {
      sync();
      // "滚动中"高亮:短暂加深,停手 600ms 后回落
      setScrolling(true);
      if (scrollTimer.current !== null) window.clearTimeout(scrollTimer.current);
      scrollTimer.current = window.setTimeout(() => setScrolling(false), 600);
    };

    // 容器自身与直接子元素都观察:scrollHeight 变化(内容增删/异步加载)
    // 只反映在子元素尺寸上,RO 观察容器只能看到 clientHeight 变化
    const ro = new ResizeObserver(sync);
    const observeAll = () => {
      ro.disconnect();
      ro.observe(el);
      for (const c of Array.from(el.children)) ro.observe(c);
    };
    // 路由切换会整棵换掉子树,childList 变了就重挂观察
    const mo = new MutationObserver(() => {
      observeAll();
      sync();
    });

    sync();
    el.addEventListener("scroll", onScroll, { passive: true });
    observeAll();
    mo.observe(el, { childList: true });
    return () => {
      el.removeEventListener("scroll", onScroll);
      ro.disconnect();
      mo.disconnect();
      if (scrollTimer.current !== null) window.clearTimeout(scrollTimer.current);
    };
  }, [targetRef]);

  const onPointerDown = (e: React.PointerEvent<HTMLDivElement>) => {
    const el = targetRef.current;
    if (!el) return;
    e.preventDefault();
    e.currentTarget.setPointerCapture(e.pointerId);
    drag.current = { startY: e.clientY, startScrollTop: el.scrollTop };
    setDragging(true);
  };
  const onPointerMove = (e: React.PointerEvent<HTMLDivElement>) => {
    const el = targetRef.current;
    const track = trackRef.current;
    if (!drag.current || !el || !track || !geo) return;
    const trackH = track.getBoundingClientRect().height;
    const usable = trackH * (1 - geo.heightPct / 100); // 轨道高 - thumb 高
    if (usable <= 0) return;
    const dy = e.clientY - drag.current.startY;
    el.scrollTop =
      drag.current.startScrollTop + (dy / usable) * (el.scrollHeight - el.clientHeight);
  };
  const endDrag = () => {
    drag.current = null;
    setDragging(false);
  };

  if (!geo) return null;
  return (
    <div ref={trackRef} className={`${styles.track} ${className ?? ""}`} aria-hidden>
      <div
        className={styles.thumb}
        data-active={dragging || scrolling || undefined}
        style={{ top: `${geo.topPct}%`, height: `${geo.heightPct}%` }}
        onPointerDown={onPointerDown}
        onPointerMove={onPointerMove}
        onPointerUp={endDrag}
        onPointerCancel={endDrag}
      />
    </div>
  );
}
