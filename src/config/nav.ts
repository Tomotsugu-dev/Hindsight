import { History, Sparkles, Cloud, MessageSquare, Settings, LayoutGrid } from "lucide-react";
import { CalendarTodayIcon } from "../components/icons/CalendarTodayIcon";
import { CalendarWeekIcon } from "../components/icons/CalendarWeekIcon";
import { CalendarMonthIcon } from "../components/icons/CalendarMonthIcon";
import type { NavItem } from "../types/nav";

export const ROUTES = {
  today: "/",
  week: "/week",
  month: "/month",
  history: "/history",
  chat: "/chat",
  aiSummary: "/ai",
  aiSettings: "/ai/settings",
  devices: "/devices",
  categories: "/categories",
  /** 旧的独立「应用」页;现已并入 /categories 的 tab,这里只留给重定向用 */
  apps: "/apps",
  settings: "/settings",
  /** A plugin page in the main window: /plugins/<plugin id> */
  plugin: "/plugins",
  /** The plugin market and installed plugins */
  settingsPlugins: "/settings/plugins",
} as const;

// 注：label 已改为 labelKey（i18n 翻译键），渲染时由组件通过 t() 解析
export const NAV_ITEMS: NavItem[] = [
  {
    path: ROUTES.today,
    labelKey: "nav.items.today",
    icon: CalendarTodayIcon,
    group: "primary",
    color: "#f97316",
    end: true,
  },
  {
    path: ROUTES.week,
    labelKey: "nav.items.week",
    icon: CalendarWeekIcon,
    group: "primary",
    color: "#3b82f6",
  },
  {
    path: ROUTES.month,
    labelKey: "nav.items.month",
    icon: CalendarMonthIcon,
    group: "primary",
    color: "#8b5cf6",
  },
  {
    path: ROUTES.history,
    labelKey: "nav.items.history",
    icon: History,
    group: "primary",
    color: "#14b8a6",
  },
  {
    path: ROUTES.chat,
    labelKey: "nav.items.chat",
    icon: MessageSquare,
    group: "ai",
    color: "#ec4899",
  },
  {
    path: ROUTES.aiSummary,
    labelKey: "nav.items.aiSummary",
    icon: Sparkles,
    group: "ai",
    color: "#d946ef",
  },
  {
    path: ROUTES.categories,
    labelKey: "nav.items.categories",
    icon: LayoutGrid,
    group: "data",
    color: "#0ea5e9",
  },
  {
    path: ROUTES.devices,
    labelKey: "nav.items.devices",
    icon: Cloud,
    group: "data",
    color: "#10b981",
  },
  {
    path: ROUTES.settings,
    labelKey: "nav.items.settings",
    icon: Settings,
    group: "system",
    color: "#64748b",
  },
];
