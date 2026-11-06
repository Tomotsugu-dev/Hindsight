import { useEffect, useMemo, useState } from "react";
import type { AppUsage } from "../api/hindsight";
import type { RingFocusApp } from "../components/PieView/ringItems";
import { useCategories } from "../state/categories";
import { secsIn } from "../utils/appCategoryTime";
import { displayAppName } from "../utils/displayName";

/** 鼠标在一行上停这么久才算选中它；快速扫过一串应用时圆环不跟着乱跳 */
const SETTLE_MS = 120;

interface Args {
  /** 鼠标停着的应用行的 id（分组 ID）；null = 没停在任何应用上 */
  hoverId: string | null;
  /** 占比视图才突出应用；时段视图下传 false */
  enabled: boolean;
  /** 本期每个应用的时长，按时长从多到少 */
  apps: AppUsage[];
}

export interface AppFocus {
  /** 交给圆环：展开哪个应用；null = 没突出应用 */
  focusApp: RingFocusApp | null;
  /** The app the stats cards narrow down to; null when no app is picked out. */
  app: AppUsage | null;
  /** 统计卡片左边色条用的颜色 */
  color?: string;
}

/** 鼠标停在应用排行某一行时，算出圆环要展开的这个应用。 */
export function useAppFocus({ hoverId, enabled, apps }: Args): AppFocus {
  const { getCategory } = useCategories();

  // 停住 SETTLE_MS 才换；从一行移到下一行中间那一下 null 也就跳过了
  const [settledId, setSettledId] = useState<string | null>(null);
  useEffect(() => {
    const timer = window.setTimeout(() => setSettledId(hoverId), SETTLE_MS);
    return () => window.clearTimeout(timer);
  }, [hoverId]);

  const app = enabled && settledId ? (apps.find((a) => a.groupId === settledId) ?? null) : null;

  return useMemo<AppFocus>(() => {
    if (!app) return { focusApp: null, app: null };

    const cat = getCategory(app.categoryId);
    const color = cat?.color ?? "#94a3b8";
    // Share of its category: only the app's time in that category counts, since website rules
    // can move part of a browser's time to other categories.
    const ownCat = new Set([app.categoryId]);
    const ownMinutes = Math.round(secsIn(app.byCategory, ownCat) / 60);

    const focusApp: RingFocusApp = {
      groupId: app.groupId,
      name: displayAppName(app.displayName),
      iconProcess: app.iconProcess,
      categoryId: app.categoryId,
      color,
      minutes: app.minutes,
      categoryMinutes: ownMinutes,
    };
    return { focusApp, app, color };
  }, [app, getCategory]);
}
