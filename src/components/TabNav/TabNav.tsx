import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { NavLink, useLocation } from "react-router-dom";
import { useTranslation } from "react-i18next";
import styles from "./TabNav.module.css";

/**
 * 通用浮动胶囊 tab 条：路由切换时弹簧滑动到激活项。
 * Settings / AISummary / AISettings 三处共用，避免逐页复制 ~70 行 pill 测量逻辑。
 *
 * 语义说明：使用 <nav> + NavLink 而非 ARIA tablist，
 * 因为 react-router 的 aria-current="page" 已能表达激活项；
 * 强制套 role="tablist" 反而会让链接的语义被屏蔽。
 */

export interface TabDef {
  /** 路由相对路径，"" 表示父路由本身 */
  to: string;
  /** i18n key，渲染时通过 t(...) 解析 */
  labelKey: string;
  /** 是否需要精确匹配（对父路由空 to 必须 true） */
  end?: boolean;
}

interface TabNavProps {
  tabs: TabDef[];
  /** 用于 nav 的 aria-label（无障碍） */
  ariaLabel?: string;
}

interface PillStyle {
  left: number;
  width: number;
  visible: boolean;
}

export function TabNav({ tabs, ariaLabel }: TabNavProps) {
  const { t, i18n } = useTranslation();
  const location = useLocation();
  const navRef = useRef<HTMLElement | null>(null);
  const [pill, setPill] = useState<PillStyle>({ left: 0, width: 0, visible: false });
  const [animated, setAnimated] = useState(false);

  /** 路由或语言变化时重新测量当前激活 tab 的位置
   *  —— 切换语言会改 tab 文字宽度，必须重测，否则 pill 留在旧尺寸 */
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
    <nav className={styles.tabs} aria-label={ariaLabel} ref={navRef}>
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

      {tabs.map((tab) => (
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
  );
}
