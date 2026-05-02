import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { useLocation } from "react-router-dom";
import { NAV_ITEMS, ROUTES } from "../../config/nav";
import type { NavGroup } from "../../types/nav";
import { NavItem } from "./NavItem";
import { StatusFooter } from "./StatusFooter";
import styles from "./Sidebar.module.css";

const GROUP_TITLE: Record<NavGroup, string> = {
  primary: "概览",
  system: "系统",
};

interface PillStyle {
  top: number;
  height: number;
  visible: boolean;
}

export function Sidebar() {
  const groups: NavGroup[] = ["primary", "system"];
  const location = useLocation();

  const navRef = useRef<HTMLElement | null>(null);
  const [pill, setPill] = useState<PillStyle>({ top: 0, height: 0, visible: false });
  const [animated, setAnimated] = useState(false);

  /** 路由变化 / 挂载时测量当前激活项位置 */
  useLayoutEffect(() => {
    const nav = navRef.current;
    if (!nav) return;

    const active = nav.querySelector<HTMLElement>('[aria-current="page"]');
    if (!active) {
      setPill((p) => ({ ...p, visible: false }));
      return;
    }

    setPill({
      top: active.offsetTop,
      height: active.offsetHeight,
      visible: true,
    });
  }, [location.pathname]);

  /** 第一次定位完成后开启过渡，避免初始从 0 滑下来 */
  useEffect(() => {
    if (pill.visible && !animated) {
      const id = requestAnimationFrame(() => setAnimated(true));
      return () => cancelAnimationFrame(id);
    }
  }, [pill.visible, animated]);

  return (
    <aside className={styles.sidebar}>
      <div className={styles.brand} data-tauri-drag-region>
        <div className={styles.logoMark} aria-hidden />
        <span className={styles.logoText}>Hindsight</span>
      </div>

      <nav className={styles.nav} ref={navRef}>
        {/* 浮动胶囊 — 在所有导航项之下 */}
        <div
          className={`${styles.pill} ${animated ? styles.pillAnimated : ""}`}
          style={{
            transform: `translate3d(0, ${pill.top}px, 0)`,
            height: pill.height,
            opacity: pill.visible ? 1 : 0,
          }}
          aria-hidden
        />

        {groups.map((group) => {
          const items = NAV_ITEMS.filter((item) => item.group === group);
          if (items.length === 0) return null;

          return (
            <div key={group} className={styles.section}>
              <div className={styles.sectionTitle}>{GROUP_TITLE[group]}</div>
              <div className={styles.sectionItems}>
                {items.map((item) => (
                  <NavItem
                    key={item.path}
                    to={item.path}
                    label={item.label}
                    Icon={item.icon}
                    color={item.color}
                    end={item.path === ROUTES.today}
                  />
                ))}
              </div>
            </div>
          );
        })}
      </nav>

      <StatusFooter />
    </aside>
  );
}
