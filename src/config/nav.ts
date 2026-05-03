import {
  LayoutDashboard,
  CalendarDays,
  CalendarRange,
  Sparkles,
  Network,
  Settings,
  Tags,
} from "lucide-react";
import type { NavItem } from "../types/nav";

export const ROUTES = {
  today: "/",
  week: "/week",
  month: "/month",
  ai: "/ai",
  devices: "/devices",
  categories: "/categories",
  settings: "/settings",
} as const;

export const NAV_ITEMS: NavItem[] = [
  { path: ROUTES.today,      label: "今日总览", icon: LayoutDashboard, group: "primary", color: "#f97316" },
  { path: ROUTES.week,       label: "周统计",   icon: CalendarDays,    group: "primary", color: "#3b82f6" },
  { path: ROUTES.month,      label: "月统计",   icon: CalendarRange,   group: "primary", color: "#8b5cf6" },
  { path: ROUTES.ai,         label: "AI 总结",  icon: Sparkles,        group: "primary", color: "#d946ef" },
  { path: ROUTES.devices,    label: "设备",     icon: Network,         group: "system",  color: "#10b981" },
  { path: ROUTES.categories, label: "应用分类", icon: Tags,            group: "system",  color: "#0ea5e9" },
  { path: ROUTES.settings,   label: "设置",     icon: Settings,        group: "system",  color: "#64748b" },
];
