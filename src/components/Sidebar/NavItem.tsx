import type { CSSProperties } from "react";
import { Link, useLocation } from "react-router-dom";
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
  /** 这些路径前缀不算激活；用于 /ai 排除 /ai/settings */
  excludePaths?: string[];
}

export function NavItem({ to, label, Icon, color, end, excludePaths }: NavItemProps) {
  const location = useLocation();
  const path = location.pathname;
  const cssVar = { "--icon-color": color } as CSSProperties;

  // 默认走 NavLink 等价的语义：end=true → 严格相等；否则 startsWith。
  // 多套一层 excludePaths：路径命中前缀匹配，但落在 excludePaths 里的子路径不算
  // ——这是为了支持 /ai 高亮 /ai/week / /ai/debug 但排除兄弟项 /ai/settings。
  // 用 Link + 手动 aria-current 而不是 NavLink，保证 Sidebar 的 querySelector
  // 跟我们这里的 isActive 计算 100% 一致。
  let isActive: boolean;
  if (excludePaths && excludePaths.length > 0) {
    const isPrefix = path === to || path.startsWith(to + "/");
    const inExclude = excludePaths.some(
      (e) => path === e || path.startsWith(e + "/"),
    );
    isActive = isPrefix && !inExclude;
  } else if (end) {
    isActive = path === to;
  } else {
    isActive = path === to || path.startsWith(to + "/");
  }

  return (
    <Link
      to={to}
      style={cssVar}
      className={`${styles.item} ${isActive ? styles.active : ""}`}
      aria-current={isActive ? "page" : undefined}
    >
      <span className={styles.iconWrap} aria-hidden>
        <Icon className={styles.icon} size={18} strokeWidth={1.85} />
      </span>
      <span className={styles.label} data-sb-label>
        {label}
      </span>
    </Link>
  );
}
