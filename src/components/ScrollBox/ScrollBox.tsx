import {
  useEffect,
  useRef,
  useState,
  type CSSProperties,
  type ReactNode,
} from "react";
import styles from "./ScrollBox.module.css";

interface ScrollBoxProps {
  children: ReactNode;
  maxHeight: number;
  /** 上下渐隐区域的高度（px） */
  fadeSize?: number;
  className?: string;
}

/**
 * 固定最大高度的滚动盒：上下两端用 mask-image 渐隐文字；
 * 仅在那个方向还能滚动时才应用渐隐（顶端无更多内容时不淡顶部）。
 */
export function ScrollBox({
  children,
  maxHeight,
  fadeSize = 24,
  className,
}: ScrollBoxProps) {
  const ref = useRef<HTMLDivElement>(null);
  const [atTop, setAtTop] = useState(true);
  const [atBottom, setAtBottom] = useState(true);

  useEffect(() => {
    const el = ref.current;
    if (!el) return;

    const update = () => {
      setAtTop(el.scrollTop <= 1);
      setAtBottom(el.scrollTop + el.clientHeight >= el.scrollHeight - 1);
    };

    update();
    el.addEventListener("scroll", update, { passive: true });
    const ro = new ResizeObserver(update);
    ro.observe(el);

    return () => {
      el.removeEventListener("scroll", update);
      ro.disconnect();
    };
  }, []);

  const cls = [
    styles.box,
    atTop ? styles.atTop : "",
    atBottom ? styles.atBottom : "",
    className ?? "",
  ]
    .filter(Boolean)
    .join(" ");

  return (
    <div
      ref={ref}
      className={cls}
      style={
        {
          height: `${maxHeight}px`,
          minHeight: `${maxHeight}px`,
          "--fade-size": `${fadeSize}px`,
        } as CSSProperties
      }
    >
      {children}
    </div>
  );
}
