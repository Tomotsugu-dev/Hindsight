import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { useLocation } from "react-router-dom";
import { NAV_ITEMS } from "../../config/nav";
import type { NavGroup } from "../../types/nav";
import { NavItem } from "./NavItem";
import { StatusFooter } from "./StatusFooter";
import { useCaptureStatus } from "../../hooks/useCaptureStatus";
import logoUrl from "../../assets/logo.png";
import styles from "./Sidebar.module.css";

const GROUP_TITLE: Record<NavGroup, string> = {
  primary: "概览",
  ai: "AI",
  system: "系统",
};

interface PillStyle {
  top: number;
  height: number;
  visible: boolean;
}

export function Sidebar() {
  const groups: NavGroup[] = ["primary", "ai", "system"];
  const location = useLocation();
  const { status, toggle } = useCaptureStatus();

  const captureUI: "ok" | "idle" | "error" = !status
    ? "ok"
    : status.lastError
      ? "error"
      : status.running
        ? "ok"
        : "idle";

  const navRef = useRef<HTMLElement | null>(null);
  const [pill, setPill] = useState<PillStyle>({ top: 0, height: 0, visible: false });
  const [animated, setAnimated] = useState(false);

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

  useEffect(() => {
    if (pill.visible && !animated) {
      const id = requestAnimationFrame(() => setAnimated(true));
      return () => cancelAnimationFrame(id);
    }
  }, [pill.visible, animated]);

  return (
    <aside className={styles.sidebar}>
      <div className={styles.brand} data-tauri-drag-region>
        <img
          className={styles.logoMark}
          src={logoUrl}
          alt=""
          aria-hidden
          draggable={false}
        />
        <span className={styles.logoText}>Hindsight</span>
      </div>

      <nav className={styles.nav} ref={navRef}>
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
                    end={item.end}
                  />
                ))}
              </div>
            </div>
          );
        })}
      </nav>

      <StatusFooter captureStatus={captureUI} onToggleCapture={toggle} />
    </aside>
  );
}
