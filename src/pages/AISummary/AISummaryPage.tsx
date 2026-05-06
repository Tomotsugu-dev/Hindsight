import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { NavLink, Outlet, useLocation } from "react-router-dom";
import styles from "./AISummaryPage.module.css";

/** Tab 配置：5 个子路由对应 5 个 tab。结构和 SettingsPage 的 TABS 一致。 */
const TABS = [
  { to: "", label: "日报", end: true },
  { to: "week", label: "周报" },
  { to: "month", label: "月报" },
  { to: "chat", label: "对话" },
  { to: "debug", label: "调试" },
];

interface PillStyle {
  left: number;
  width: number;
  visible: boolean;
}

/**
 * AI 总结页外壳：标题 + 5 个 tab + Outlet。
 *
 * 跟 [SettingsPage] 完全对齐——浮动胶囊指示当前激活 tab，路由切换时弹簧滑动。
 * 子路由各自实现内容（DailyTab 是真主体，其它是占位）。
 */
export default function AISummaryPage() {
  const location = useLocation();
  const navRef = useRef<HTMLElement | null>(null);
  const [pill, setPill] = useState<PillStyle>({
    left: 0,
    width: 0,
    visible: false,
  });
  const [animated, setAnimated] = useState(false);

  /** 路由变化时重新测量当前激活 tab 的位置——跟 SettingsPage 完全一致 */
  useLayoutEffect(() => {
    const nav = navRef.current;
    if (!nav) return;

    const active = nav.querySelector<HTMLElement>('[aria-current="page"]');
    if (!active) {
      setPill((p) => ({ ...p, visible: false }));
      return;
    }

    setPill({
      left: active.offsetLeft,
      width: active.offsetWidth,
      visible: true,
    });
  }, [location.pathname]);

  /** 第一次定位完成后再开启过渡，避免初始从 0 滑过来 */
  useEffect(() => {
    if (pill.visible && !animated) {
      const id = requestAnimationFrame(() => setAnimated(true));
      return () => cancelAnimationFrame(id);
    }
  }, [pill.visible, animated]);

  return (
    <div className={styles.page}>
      <header className={styles.header}>
        <h1 className={styles.title}>AI 总结</h1>
      </header>

      <nav className={styles.tabs} role="tablist" ref={navRef}>
        {/* 浮动胶囊 — 位于所有 tab 之下 */}
        <div
          className={`${styles.pill} ${animated ? styles.pillAnimated : ""}`}
          style={{
            transform: `translate3d(${pill.left}px, 0, 0)`,
            width: pill.width,
            opacity: pill.visible ? 1 : 0,
          }}
          aria-hidden
        />

        {TABS.map((t) => (
          <NavLink
            key={t.label}
            to={t.to}
            end={t.end}
            className={({ isActive }) =>
              `${styles.tab} ${isActive ? styles.tabActive : ""}`
            }
          >
            {t.label}
          </NavLink>
        ))}
      </nav>

      <section className={styles.tabContent}>
        <Outlet />
      </section>
    </div>
  );
}
