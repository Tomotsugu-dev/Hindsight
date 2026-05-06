import type { LucideIcon } from "lucide-react";

export type NavGroup = "primary" | "ai" | "system";

export interface NavItem {
  /** 路由路径 */
  path: string;
  /** i18n 翻译 key（在渲染时通过 t() 解析） */
  labelKey: string;
  /** 图标组件 */
  icon: LucideIcon;
  /** 所属分组 */
  group: NavGroup;
  /** 图标主题色（任意 CSS color） */
  color: string;
  /** NavLink end 匹配；用于路径会被其他子路由前缀命中的项（如 /） */
  end?: boolean;
  /** 这些路径前缀算"不属于本项"；用于 /ai 想匹配 /ai/week 等子页但要排除 /ai/settings */
  excludePaths?: string[];
}
