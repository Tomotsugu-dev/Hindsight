import type { LucideIcon } from "lucide-react";

export type NavGroup = "primary" | "system";

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
}
