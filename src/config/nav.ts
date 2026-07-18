import {
  Sparkles,
  Cloud,
  MessageSquare,
  ScanSearch,
  Settings,
  LayoutGrid,
  Layers,
} from "lucide-react";
import { CalendarTodayIcon } from "../components/icons/CalendarTodayIcon";
import { CalendarWeekIcon } from "../components/icons/CalendarWeekIcon";
import { CalendarMonthIcon } from "../components/icons/CalendarMonthIcon";
import { AiSettingsIcon } from "../components/icons/AiSettingsIcon";
import type { NavItem } from "../types/nav";

export const ROUTES = {
  today: "/",
  week: "/week",
  month: "/month",
  chat: "/chat",
  search: "/search",
  aiSummary: "/ai",
  aiSettings: "/ai/settings",
  devices: "/devices",
  categories: "/categories",
  apps: "/apps",
  settings: "/settings",
} as const;

// 注：label 已改为 labelKey（i18n 翻译键），渲染时由组件通过 t() 解析
export const NAV_ITEMS: NavItem[] = [
  { path: ROUTES.today,      labelKey: "nav.items.today",      icon: CalendarTodayIcon, group: "primary", color: "#f97316", end: true },
  { path: ROUTES.week,       labelKey: "nav.items.week",       icon: CalendarWeekIcon,  group: "primary", color: "#3b82f6" },
  { path: ROUTES.month,      labelKey: "nav.items.month",      icon: CalendarMonthIcon, group: "primary", color: "#8b5cf6" },
  // /ai 是 AI 总结的根；子页 /ai/week / /ai/debug 也应该让 AI 总结高亮，
  // 但 /ai/settings 是兄弟项（AI 设置）——用 excludePaths 把它从前缀匹配里抠掉
  { path: ROUTES.chat,       labelKey: "nav.items.chat",       icon: MessageSquare,     group: "ai",      color: "#ec4899" },
  { path: ROUTES.search,     labelKey: "nav.items.search",     icon: ScanSearch,        group: "ai",      color: "#06b6d4" },
  { path: ROUTES.aiSummary,  labelKey: "nav.items.aiSummary",  icon: Sparkles,          group: "ai",      color: "#d946ef", excludePaths: [ROUTES.aiSettings] },
  { path: ROUTES.aiSettings, labelKey: "nav.items.aiSettings", icon: AiSettingsIcon,    group: "ai",      color: "#a855f7" },
  { path: ROUTES.categories, labelKey: "nav.items.categories", icon: LayoutGrid,        group: "data",    color: "#0ea5e9" },
  { path: ROUTES.apps,       labelKey: "nav.items.apps",       icon: Layers,            group: "data",    color: "#14b8a6" },
  { path: ROUTES.devices,    labelKey: "nav.items.devices",    icon: Cloud,             group: "data",    color: "#10b981" },
  { path: ROUTES.settings,   labelKey: "nav.items.settings",   icon: Settings,          group: "system",  color: "#64748b" },
];
