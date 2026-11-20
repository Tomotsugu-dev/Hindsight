/**
 * 「AI 能看到的数据」配置：读一个功能的配置，记住页面上选的那套，保存改动。
 *
 * 页面上的选择不存：每次进页面回到默认。日报、周报默认是「自动生成时用」的那套，
 * 聊天默认用默认分类。选中的那套被删了，也回到默认。
 */

import { useState } from "react";
import type {
  AiProfileModule,
  AiVisibility,
  ModuleProfiles,
  VisibilityProfile,
} from "../api/hindsight";
import { useSettings } from "./settings";

const EMPTY: ModuleProfiles = { profiles: {}, autoProfile: null };

export interface AiProfilesState {
  module: AiProfileModule;
  /** 这个功能的全部配置 */
  current: ModuleProfiles;
  /** 按名字排好的 [id, 配置] */
  list: [string, VisibilityProfile][];
  /** 页面上选中的配置 id；null = 默认分类 */
  selected: string | null;
  select: (id: string | null) => void;
  /** 存这个功能的全部配置 */
  save: (next: ModuleProfiles) => void;
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
  current: ModuleProfiles,
  picked: string | null | undefined,
): string | null {
  const exists = (id: string | null | undefined): id is string =>
    id != null && Object.prototype.hasOwnProperty.call(current.profiles, id);
  const fallback = module !== "chat" && exists(current.autoProfile) ? current.autoProfile : null;
  if (picked === undefined) return fallback;
  if (picked === null) return null;
  return exists(picked) ? picked : fallback;
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

export function useAiProfiles(module: AiProfileModule): AiProfilesState {
  const { settings, update } = useSettings();
  const visibility: Partial<AiVisibility> = settings?.ai.visibility ?? {};
  const current = visibility[module] ?? EMPTY;
  // undefined = 用户还没选过，跟着默认走
  const [picked, setPicked] = useState<string | null | undefined>(undefined);

  const save = (next: ModuleProfiles) => {
    if (!settings) return;
    const ai = settings.ai;
    // 老版本存的设置可能没有这一块，缺的功能补成空的
    const old: Partial<AiVisibility> = ai.visibility ?? {};
    const filled: AiVisibility = {
      daily: old.daily ?? EMPTY,
      weekly: old.weekly ?? EMPTY,
      chat: old.chat ?? EMPTY,
    };
    update({ ai: { ...ai, visibility: { ...filled, [module]: next } } });
  };

  return {
    module,
    current,
    list: sortProfiles(current.profiles),
    selected: resolveSelected(module, current, picked),
    select: setPicked,
    save,
  };
}
