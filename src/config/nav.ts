import {
  LayoutDashboard,
  CalendarDays,
  CalendarRange,
  Sparkles,
  Cloud,
  Settings,
} from "lucide-react";
import type { NavItem } from "../types/nav";

/** 路由路径常量 */
export const ROUTES = {
  today: "/",
  week: "/week",
  month: "/month",
  ai: "/ai",
  sync: "/sync",
  settings: "/settings",
} as const;

/** 导航项单一数据源（侧边栏渲染 + 路由生成都从这里来） */
export const NAV_ITEMS: NavItem[] = [
  { path: ROUTES.today,    label: "今日总览", icon: LayoutDashboard, group: "primary", color: "#f97316" }, // 橘
  { path: ROUTES.week,     label: "周统计",   icon: CalendarDays,    group: "primary", color: "#3b82f6" }, // 蓝
  { path: ROUTES.month,    label: "月统计",   icon: CalendarRange,   group: "primary", color: "#8b5cf6" }, // 紫
  { path: ROUTES.ai,       label: "AI 总结",  icon: Sparkles,        group: "primary", color: "#d946ef" }, // 品红
  { path: ROUTES.sync,     label: "同步",     icon: Cloud,           group: "system",  color: "#06b6d4" }, // 青
  { path: ROUTES.settings, label: "设置",     icon: Settings,        group: "system",  color: "#64748b" }, // 石板灰
];
