import type { CSSProperties } from "react";
import { Link, useLocation } from "react-router-dom";
import type { LucideIcon } from "lucide-react";
import styles from "./NavItem.module.css";

interface NavItemProps {
  to: string;
  label: string;
  Icon: LucideIcon;
  /** An image shown instead of `Icon`, such as a plugin's own icon */
  iconSrc?: string;
  /** 图标主题色 */
  color: string;
  /** 路径是否完全匹配（用于首页 "/"）*/
  end?: boolean;
  /** Short text after the label, such as 「新」 */
  badge?: string;
  /** When set, the item is a button that runs this instead of a link to `to`. Used by plugins
   * whose page opens in its own window */
  onClick?: () => void;
}

export function NavItem({ to, label, Icon, iconSrc, color, end, badge, onClick }: NavItemProps) {
  const location = useLocation();
  const path = location.pathname;
  const cssVar = { "--icon-color": color } as CSSProperties;

  // 默认走 NavLink 等价的语义：end=true → 严格相等；否则 startsWith。
  // 用 Link + 手动 aria-current 而不是 NavLink，保证 Sidebar 的 querySelector
  // 跟我们这里的 isActive 计算 100% 一致。
  let isActive: boolean;
  if (onClick) {
    isActive = false;
  } else if (end) {
    isActive = path === to;
  } else {
    isActive = path === to || path.startsWith(to + "/");
  }

  const content = (
    <>
      <span className={styles.iconWrap} aria-hidden>
        {iconSrc ? (
          <img className={styles.image} src={iconSrc} alt="" />
        ) : (
          <Icon className={styles.icon} size={18} strokeWidth={1.85} />
        )}
      </span>
      <span className={styles.label} data-sb-label>
        {label}
      </span>
      {badge ? <span className={styles.badge}>{badge}</span> : null}
    </>
  );

  if (onClick) {
    return (
      <button
        type="button"
        style={cssVar}
        className={`${styles.item} ${styles.button}`}
        data-part="nav-item"
        onClick={onClick}
      >
        {content}
      </button>
    );
  }
  return (
    <Link
      to={to}
      style={cssVar}
      className={`${styles.item} ${isActive ? styles.active : ""}`}
      data-part="nav-item"
      aria-current={isActive ? "page" : undefined}
    >
      {content}
    </Link>
  );
}
