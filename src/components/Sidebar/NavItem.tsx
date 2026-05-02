import type { CSSProperties } from "react";
import { NavLink } from "react-router-dom";
import type { LucideIcon } from "lucide-react";
import styles from "./NavItem.module.css";

interface NavItemProps {
  to: string;
  label: string;
  Icon: LucideIcon;
  /** 图标主题色 */
  color: string;
  /** 路径是否完全匹配（用于首页 "/"）*/
  end?: boolean;
}

export function NavItem({ to, label, Icon, color, end }: NavItemProps) {
  const cssVar = { "--icon-color": color } as CSSProperties;

  return (
    <NavLink
      to={to}
      end={end}
      style={cssVar}
      className={({ isActive }) =>
        `${styles.item} ${isActive ? styles.active : ""}`
      }
    >
      <span className={styles.iconWrap} aria-hidden>
        <Icon className={styles.icon} size={18} strokeWidth={1.85} />
      </span>
      <span className={styles.label}>{label}</span>
    </NavLink>
  );
}
