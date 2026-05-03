import {
  BookOpen,
  Brush,
  Briefcase,
  Camera,
  Code,
  Coffee,
  Compass,
  Dumbbell,
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
  Tag,
};

export const ICON_NAMES = Object.keys(CATEGORY_ICONS);

export function resolveCategoryIcon(name: string | undefined | null): LucideIcon {
  if (!name) return Tag;
  return CATEGORY_ICONS[name] ?? Tag;
}
