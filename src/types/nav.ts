import type { LucideIcon } from "lucide-react";

export type NavGroup = "primary" | "ai" | "system";

export interface NavItem {
  /** 路由路径 */
  path: string;
  /** 显示标题 */
  label: string;
  /** 图标组件 */
  icon: LucideIcon;
  /** 所属分组 */
  group: NavGroup;
  /** 图标主题色（任意 CSS color） */
  color: string;
  /** NavLink end 匹配；用于路径会被其他子路由前缀命中的项（如 / 与 /ai） */
  end?: boolean;
}
