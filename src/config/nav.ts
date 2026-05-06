import {
  LayoutDashboard,
  CalendarDays,
  CalendarRange,
  Sparkles,
  BrainCircuit,
  Cloud,
  Settings,
  Boxes,
} from "lucide-react";
import type { NavItem } from "../types/nav";

export const ROUTES = {
  today: "/",
  week: "/week",
  month: "/month",
  aiSummary: "/ai",
  aiSettings: "/ai/settings",
  devices: "/devices",
  categories: "/categories",
  settings: "/settings",
} as const;

// 注：label 已改为 labelKey（i18n 翻译键），渲染时由组件通过 t() 解析
export const NAV_ITEMS: NavItem[] = [
  { path: ROUTES.today,      labelKey: "nav.items.today",      icon: LayoutDashboard, group: "primary", color: "#f97316", end: true },
  { path: ROUTES.week,       labelKey: "nav.items.week",       icon: CalendarDays,    group: "primary", color: "#3b82f6" },
  { path: ROUTES.month,      labelKey: "nav.items.month",      icon: CalendarRange,   group: "primary", color: "#8b5cf6" },
  // /ai 是 AI 总结的根；子页 /ai/week / /ai/debug 也应该让 AI 总结高亮，
  // 但 /ai/settings 是兄弟项（AI 设置）——用 excludePaths 把它从前缀匹配里抠掉
  { path: ROUTES.aiSummary,  labelKey: "nav.items.aiSummary",  icon: Sparkles,        group: "ai",      color: "#d946ef", excludePaths: [ROUTES.aiSettings] },
  { path: ROUTES.aiSettings, labelKey: "nav.items.aiSettings", icon: BrainCircuit,    group: "ai",      color: "#a855f7" },
  { path: ROUTES.categories, labelKey: "nav.items.categories", icon: Boxes,           group: "system",  color: "#0ea5e9" },
  { path: ROUTES.devices,    labelKey: "nav.items.devices",    icon: Cloud,           group: "system",  color: "#10b981" },
  { path: ROUTES.settings,   labelKey: "nav.items.settings",   icon: Settings,        group: "system",  color: "#64748b" },
];
