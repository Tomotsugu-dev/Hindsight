import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { NavLink, Outlet, useLocation } from "react-router-dom";
import { useTranslation } from "react-i18next";
import styles from "./SettingsPage.module.css";

// tab 路由元数据；label 通过 t() 动态解析
const TABS = [
  { to: "", labelKey: "settings.tabs.general", end: true },
  { to: "data", labelKey: "settings.tabs.data" },
  { to: "privacy", labelKey: "settings.tabs.privacy" },
  { to: "about", labelKey: "settings.tabs.about" },
];

interface PillStyle {
  left: number;
  width: number;
  visible: boolean;
}

export default function SettingsPage() {
  const { t, i18n } = useTranslation();
  const location = useLocation();
  const navRef = useRef<HTMLElement | null>(null);
  const [pill, setPill] = useState<PillStyle>({ left: 0, width: 0, visible: false });
  const [animated, setAnimated] = useState(false);

  /** 路由或语言变化时重新测量当前激活 tab 的位置
   *  —— 切换语言会改 tab 文字宽度，必须重测，否则 pill 留在旧（中文）尺寸 */
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
  }, [location.pathname, i18n.language]);

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
        <h1 className={styles.title}>{t("settings.pageTitle")}</h1>
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

        {TABS.map((tab) => (
          <NavLink
            key={tab.labelKey}
            to={tab.to}
            end={tab.end}
            className={({ isActive }) =>
              `${styles.tab} ${isActive ? styles.tabActive : ""}`
            }
          >
            {t(tab.labelKey)}
          </NavLink>
        ))}
      </nav>

      <section className={styles.tabContent}>
        <Outlet />
      </section>
    </div>
  );
}
