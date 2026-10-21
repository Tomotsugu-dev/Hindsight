// Icons on the island: the category icons, plus a few for buttons.
import {
  Check,
  Minus,
  Pause,
  Play,
  Plus,
  RotateCcw,
  SkipForward,
  Square,
  X,
  type LucideIcon,
} from "lucide-react";
import { resolveCategoryIcon } from "../config/categoryIcons";

const BUTTON_ICONS: Record<string, LucideIcon> = {
  Play,
  Pause,
  Square,
  SkipForward,
  RotateCcw,
  Check,
  X,
  Plus,
  Minus,
};

/** The icon called `name`. A name this version does not know gets the default category icon. */
export function islandIcon(name: string): LucideIcon {
  return BUTTON_ICONS[name] ?? resolveCategoryIcon(name);
}
