/**
 * AI profiles: one set that every feature picks from.
 *
 * The pick on a page is not saved. Each time the page opens, it goes back to the default:
 * daily and weekly reports use the profile they use when they run automatically, and chat
 * uses the default categories. If the picked profile is deleted, the pick also goes back to
 * the default.
 */

import { useState } from "react";
import type {
  AiProfileModule,
  AiVisibility,
  AutoProfiles,
  VisibilityProfile,
} from "../api/hindsight";
import { ROUTES } from "../config/nav";
import { useSettings } from "./settings";

/** Opens the profiles in AI settings. The card is outlined for a moment when a link opens it. */
export const AI_PROFILES_HREF = `${ROUTES.aiSettings}/general?focus=profiles`;

export interface AiProfilesState {
  /** [id, profile], sorted by name */
  list: [string, VisibilityProfile][];
  /** The profile picked on the page; null = the default categories */
  selected: string | null;
  select: (id: string | null) => void;
}

/** 按名字排序，名字一样按 id，保证顺序稳定。 */
export function sortProfiles(profiles: Record<string, VisibilityProfile>) {
  return Object.entries(profiles).sort(
    ([ia, a], [ib, b]) => a.name.localeCompare(b.name) || ia.localeCompare(ib),
  );
}

/** 页面上该用哪套：用户选过且还在，就用它；否则回到默认。 */
export function resolveSelected(
  module: AiProfileModule,
  current: AiVisibility,
  picked: string | null | undefined,
): string | null {
  const exists = (id: string | null | undefined): id is string =>
    id != null && Object.prototype.hasOwnProperty.call(current.profiles, id);
  const auto = module === "chat" ? null : current.auto[module];
  const fallback = exists(auto) ? auto : null;
  if (picked === undefined) return fallback;
  if (picked === null) return null;
  return exists(picked) ? picked : fallback;
}

/** Returns `auto` with the reports that used profile `id` back on the default categories. */
export function clearAuto(auto: AutoProfiles, id: string): AutoProfiles {
  return {
    daily: auto.daily === id ? null : auto.daily,
    weekly: auto.weekly === id ? null : auto.weekly,
  };
}

/**
 * 保存前的检查：每套要有名字、名字不重复、至少选一个分类。
 * 返回有问题的那几套：id → 问题的 i18n key，每套只报一个。重名的几套都报。空的就能存。
 */
export function profileProblems(
  profiles: Record<string, VisibilityProfile>,
): Record<string, string> {
  const count = new Map<string, number>();
  for (const p of Object.values(profiles)) {
    const name = p.name.trim();
    count.set(name, (count.get(name) ?? 0) + 1);
  }
  const problems: Record<string, string> = {};
  for (const [id, p] of Object.entries(profiles)) {
    const name = p.name.trim();
    if (!name) problems[id] = "aiProfiles.errors.noName";
    else if ((count.get(name) ?? 0) > 1) problems[id] = "aiProfiles.errors.sameName";
    else if (p.categories.length === 0) problems[id] = "aiProfiles.errors.noCategory";
  }
  return problems;
}

/** 新建时的默认名：「配置 1」「配置 2」…，跳过已经用掉的。 */
export function nextProfileName(
  profiles: Record<string, VisibilityProfile>,
  nameOf: (n: number) => string,
): string {
  const used = new Set(Object.values(profiles).map((p) => p.name.trim()));
  let n = 1;
  while (used.has(nameOf(n))) n += 1;
  return nameOf(n);
}

/** Reads all profiles and saves them. */
export function useAiVisibility() {
  const { settings, update } = useSettings();
  // Settings from the web demo, or from an older version, may lack this block or parts of it
  const v: Partial<AiVisibility> | undefined = settings?.ai.visibility;
  const current: AiVisibility = {
    profiles: v?.profiles ?? {},
    auto: { daily: v?.auto?.daily ?? null, weekly: v?.auto?.weekly ?? null },
  };

  const save = (next: AiVisibility) => {
    if (!settings) return;
    update({ ai: { ...settings.ai, visibility: next } });
  };

  return { current, save };
}

/** The profile picker of one page: all profiles, and the one picked on this page. */
export function useAiProfiles(module: AiProfileModule): AiProfilesState {
  const { current } = useAiVisibility();
  // undefined = not picked yet, so it follows the default
  const [picked, setPicked] = useState<string | null | undefined>(undefined);

  return {
    list: sortProfiles(current.profiles),
    selected: resolveSelected(module, current, picked),
    select: setPicked,
  };
}
