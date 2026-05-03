import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { NavLink, Outlet, useLocation } from "react-router-dom";
import styles from "./SettingsPage.module.css";

const TABS = [
  { to: "", label: "常规", end: true },
  { to: "data", label: "数据" },
  { to: "about", label: "关于" },
];

interface PillStyle {
  left: number;
  width: number;
  visible: boolean;
}

export default function SettingsPage() {
  const location = useLocation();
  const navRef = useRef<HTMLElement | null>(null);
  const [pill, setPill] = useState<PillStyle>({ left: 0, width: 0, visible: false });
  const [animated, setAnimated] = useState(false);

  /** 路由变化时测量当前激活 tab 的位置 */
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
        <h1 className={styles.title}>设置</h1>
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
