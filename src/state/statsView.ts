/**
 * 「时段 / 占比」图表视图偏好的全局状态：Today/Week/Month 三页**各记各的**，
 * 按 scope 分成三个独立键，各自持久化到 localStorage。
 *
 * 提到 module level 的理由：原来 view 是各页 useState("bars") 的本地状态，
 * 切侧边栏让页面 unmount → 状态丢失，切回来复位成 "bars"，用户上次选的
 * "占比" 被忘掉。放这里后：切页往返与进程重启都保留每页上次选的视图。
 * 跟 deviceFilter 的 selectedDeviceId 同款——跨页存活的展示偏好，只是这里
 * 三页互不影响（在 Today 切占比不带动 Week/Month）。
 */

import { logWarn } from "../lib/logger";

export type StatsView = "bars" | "pie";
/** 三个统计页各自的偏好槽；决定 localStorage 键与快照读写的粒度。 */
export type StatsViewScope = "today" | "week" | "month";

const KEY_PREFIX = "hindsight.stats.view.";
const DEFAULT_VIEW: StatsView = "bars";

function isStatsView(v: unknown): v is StatsView {
  return v === "bars" || v === "pie";
}

function readInitial(scope: StatsViewScope): StatsView {
  try {
    const raw = localStorage.getItem(KEY_PREFIX + scope);
    return isStatsView(raw) ? raw : DEFAULT_VIEW;
  } catch (e) {
    logWarn("statsView.read", e);
    return DEFAULT_VIEW;
  }
}

const views: Record<StatsViewScope, StatsView> = {
  today: readInitial("today"),
  week: readInitial("week"),
  month: readInitial("month"),
};
// 单一 listener 集合：某页写入会通知全部订阅者，但各页 getSnapshot 只读自己的
// scope，值没变的页 useSyncExternalStore 会自动 bail out 不重渲染。
const listeners = new Set<() => void>();

export function subscribeStatsView(cb: () => void): () => void {
  listeners.add(cb);
  return () => {
    listeners.delete(cb);
  };
}

export function getStatsView(scope: StatsViewScope): StatsView {
  return views[scope];
}

export function setStatsView(scope: StatsViewScope, next: StatsView): void {
  if (views[scope] === next) return;
  views[scope] = next;
  try {
    localStorage.setItem(KEY_PREFIX + scope, next);
  } catch (e) {
    logWarn("statsView.write", e);
  }
  listeners.forEach((cb) => cb());
}
