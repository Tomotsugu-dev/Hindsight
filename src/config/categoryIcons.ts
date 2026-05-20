import {
  BookOpen,
  Brush,
  Briefcase,
  Camera,
  Code,
  Coffee,
  Compass,
  Dumbbell,
  EyeOff,
  Film,
  Gamepad2,
  Globe,
  GraduationCap,
  Heart,
  Image,
  Mail,
  MessageCircle,
  MessagesSquare,
  Monitor,
  MoreHorizontal,
  Music,
  Palette,
  Search,
  ShoppingCart,
  Sparkles,
  Tag,
  Terminal,
  Video,
  Wallet,
} from "lucide-react";
import type { LucideIcon } from "lucide-react";

export const CATEGORY_ICONS: Record<string, LucideIcon> = {
  Code,
  Terminal,
  Globe,
  Search,
  Compass,
  MessageCircle,
  MessagesSquare,
  Mail,
  Brush,
  Palette,
  Image,
  Camera,
  Gamepad2,
  Music,
  Film,
  Video,
  Briefcase,
  GraduationCap,
  BookOpen,
  Heart,
  Coffee,
  Dumbbell,
  ShoppingCart,
  Wallet,
  Sparkles,
  Monitor,
  MoreHorizontal,
  EyeOff,
  Tag,
};

export const ICON_NAMES = Object.keys(CATEGORY_ICONS);

export function resolveCategoryIcon(name: string | undefined | null): LucideIcon {
  if (!name) return Tag;
  return CATEGORY_ICONS[name] ?? Tag;
}

/** 分类色板：AppearancePicker 的可选色 + 新建分类的初始色都从这里取。
 *  按色相由暖到冷排列（红 → 橙 → 黄 → 绿 → 青 → 蓝 → 紫 → 灰），最后两个灰色
 *  分别用作默认分类 `other` (#94a3b8) 和 `hidden` (#64748b) 的"低存在感"标识。
 *  21 色刚好排 3×7 网格。 */
export const CATEGORY_PALETTE = [
  "#f87171", // red-400
  "#fb7185", // rose-400
  "#f43f5e", // rose-500
  "#ec4899", // pink-500
  "#d946ef", // fuchsia-500
  "#a78bfa", // violet-400
  "#6366f1", // indigo-500
  "#60a5fa", // blue-400
  "#3b82f6", // blue-500
  "#0ea5e9", // sky-500
  "#06b6d4", // cyan-500
  "#14b8a6", // teal-500
  "#34d399", // emerald-400
  "#10b981", // emerald-500
  "#84cc16", // lime-500
  "#facc15", // yellow-400
  "#fbbf24", // amber-400
  "#fb923c", // orange-400
  "#f97316", // orange-500
  "#94a3b8", // slate-400（默认给 'other'）
  "#64748b", // slate-500（默认给 'hidden'，比 other 略深以区分）
];
