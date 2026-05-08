import { useEffect, useRef, useState } from "react";
import { TabNav, type TabDef } from "./TabNav";
import styles from "./FloatingTabNav.module.css";

/**
 * TabNav 包装：原 TabNav 留在文档流跟随滚动；当它**完全滚出视口顶端**时，
 * 一个浮动副本以淡入 + 轻微下落动画在视口顶端居中漂浮显示，
 * 让用户在长内容滚动时仍能看见 / 点击 tabs。
 *
 * pill 以外的整片 wrapper 透明 + `pointer-events: none`，点击穿透到下方内容。
 * 只有 pill 本身被恢复 `pointer-events: auto`，可正常切换 tab。
 *
 * 用 IntersectionObserver 监听原 TabNav 的可见性（threshold: 0）：
 * 完全离开视口（任意方向）→ pinned=true → 浮动副本可见。
 *
 * 接口与 [TabNav](./TabNav.tsx) 完全一致，可直接替换。
 */
export interface FloatingTabNavProps {
  tabs?: TabDef[];
  groups?: TabDef[][];
  ariaLabel?: string;
}

export function FloatingTabNav(props: FloatingTabNavProps) {
  const slotRef = useRef<HTMLDivElement>(null);
  const [pinned, setPinned] = useState(false);

  useEffect(() => {
    const target = slotRef.current;
    if (!target) return;
    const observer = new IntersectionObserver(
      ([entry]) => setPinned(!entry.isIntersecting),
      { threshold: 0 },
    );
    observer.observe(target);
    return () => observer.disconnect();
  }, []);

  return (
    <>
      {/* 原 TabNav：留在文档流，跟内容一起滚动 */}
      <div ref={slotRef}>
        <TabNav {...props} />
      </div>

      {/* 浮动副本：原 TabNav 滚出视口后才淡入 */}
      <div
        className={`${styles.floatingTabs} ${pinned ? styles.floatingTabsVisible : ""}`}
        aria-hidden={!pinned}
      >
        <TabNav {...props} />
      </div>
    </>
  );
}
