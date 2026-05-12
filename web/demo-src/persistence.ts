// localStorage 持久化工具——demo 内用户改动（设置 / 分类编辑 / 分组）刷新仍在。
// 跨浏览器 / 隐身模式重置（这是 demo 的合理行为，不是 bug）。

const KEY = "hindsight-demo-state-v2";

export interface DemoState {
  // 不存"day 数据"——那是只读的 fixtures；只存用户能改的部分
  categories?: unknown;
  settings?: unknown;
  selfDevice?: unknown;
  appGroups?: unknown;
}

export const persistence = {
  load(): DemoState | null {
    try {
      const raw = localStorage.getItem(KEY);
      if (!raw) return null;
      return JSON.parse(raw) as DemoState;
    } catch {
      return null;
    }
  },

  save(state: DemoState): void {
    try {
      localStorage.setItem(KEY, JSON.stringify(state));
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
