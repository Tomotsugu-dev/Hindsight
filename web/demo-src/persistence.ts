// localStorage 持久化工具——demo 内用户改动（设置 / 分类编辑 / 分组）刷新仍在。
// 跨浏览器 / 隐身模式重置（这是 demo 的合理行为，不是 bug）。

const KEY = "hindsight-demo-state-v6";

export interface DemoState {
  // 不存"day 数据"——那是只读的 fixtures；只存用户能改的部分
  categories?: unknown;
  settings?: unknown;
  selfDevice?: unknown;
  appGroups?: unknown;
}

/**
 * A save with the shape of the demo's starting data at the time. When a later demo changes that
 * shape, an old save lacks fields that the pages read, so it is not used.
 */
interface Saved {
  shape: string;
  state: DemoState;
}

export const persistence = {
  /** Returns the saved state, or null if there is none or it was saved for another `shape`. */
  load(shape: string): DemoState | null {
    try {
      const raw = localStorage.getItem(KEY);
      if (!raw) return null;
      const saved = JSON.parse(raw) as Partial<Saved>;
      return saved.shape === shape && saved.state ? saved.state : null;
    } catch {
      return null;
    }
  },

  save(state: DemoState, shape: string): void {
    try {
      localStorage.setItem(KEY, JSON.stringify({ shape, state } satisfies Saved));
    } catch {
      // localStorage 满 / 隐身模式禁用，静默失败
    }
  },

  clear(): void {
    try {
      localStorage.removeItem(KEY);
    } catch {
      // ignore
    }
  },
};

/** Describes the field names and value types in `value`, without the values. An array is
 *  described by its first item. */
export function shapeOf(value: unknown): string {
  if (Array.isArray(value)) return `[${value.length > 0 ? shapeOf(value[0]) : ""}]`;
  if (value !== null && typeof value === "object") {
    const fields = Object.keys(value).sort();
    return `{${fields.map((k) => `${k}:${shapeOf((value as Record<string, unknown>)[k])}`).join(",")}}`;
  }
  return value === null ? "null" : typeof value;
}
