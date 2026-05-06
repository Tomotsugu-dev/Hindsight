import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { useLocation, useNavigate } from "react-router-dom";
import { useTranslation } from "react-i18next";
import { Cloud, CloudOff } from "lucide-react";
import { NAV_ITEMS, ROUTES } from "../../config/nav";
import type { NavGroup } from "../../types/nav";
import { NavItem } from "./NavItem";
import { StatusFooter } from "./StatusFooter";
import { useCaptureStatus } from "../../hooks/useCaptureStatus";
import { api, type AuthState } from "../../api/hindsight";
import logoUrl from "../../assets/logo.png";
import styles from "./Sidebar.module.css";

// 分组标题 i18n key 映射
const GROUP_TITLE_KEY: Record<NavGroup, string> = {
  primary: "nav.groups.primary",
  ai: "nav.groups.ai",
  system: "nav.groups.system",
};

interface PillStyle {
  top: number;
  height: number;
  visible: boolean;
}

export function Sidebar() {
  const groups: NavGroup[] = ["primary", "ai", "system"];
  const location = useLocation();
  const navigate = useNavigate();
  const { t } = useTranslation();
  const { status, toggle } = useCaptureStatus();

  // 云同步登录状态：周期性刷新 + 窗口聚焦时立刻刷一次
  const [auth, setAuth] = useState<AuthState | null>(null);
  useEffect(() => {
    const fetch = () => {
      api.authStatus().then(setAuth).catch(() => {});
    };
    fetch();
    const interval = window.setInterval(fetch, 60_000);
    const onFocus = () => fetch();
    window.addEventListener("focus", onFocus);
    return () => {
      window.clearInterval(interval);
      window.removeEventListener("focus", onFocus);
    };
  }, []);
  const signedIn = auth?.signedIn ?? false;
  // 同步行文案：登录后显示邮箱（缺省 fallback 已连接），未登录显示"未登录"
  const syncLabel = signedIn
    ? auth?.email ?? t("sidebar.sync.connected")
    : t("sidebar.sync.signedOut");

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

      {/* Logo 下方独立一行：账户/云同步入口（沿用 footer .row 同款样式） */}
      <button
        className={styles.accountRow}
        type="button"
        onClick={() => navigate(ROUTES.devices)}
        aria-label={t("sidebar.sync.aria")}
        title={t("sidebar.sync.title")}
      >
        <span className={styles.iconWrap} aria-hidden>
          {signedIn ? (
            <Cloud
              size={18}
              strokeWidth={1.75}
              className={`${styles.cloud} ${styles.cloudOn}`}
            />
          ) : (
            <CloudOff
              size={18}
              strokeWidth={1.75}
              className={styles.cloud}
            />
          )}
        </span>
        <span className={`${styles.text} ${signedIn ? styles.textOn : ""}`}>
          {syncLabel}
        </span>
      </button>

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
              <div className={styles.sectionTitle}>{t(GROUP_TITLE_KEY[group])}</div>
              <div className={styles.sectionItems}>
                {items.map((item) => (
                  <NavItem
                    key={item.path}
                    to={item.path}
                    label={t(item.labelKey)}
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
