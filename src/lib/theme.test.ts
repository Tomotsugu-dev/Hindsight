// node 环境手工桩:theme.ts 只用到 localStorage 与 documentElement.dataset,
// 不必为此挂 jsdom(vitest 配置保持纯函数策略)。
import { beforeEach, describe, expect, it, vi } from "vitest";

const store = new Map<string, string>();
vi.stubGlobal("localStorage", {
  getItem: (k: string) => store.get(k) ?? null,
  setItem: (k: string, v: string) => void store.set(k, v),
  removeItem: (k: string) => void store.delete(k),
});
vi.stubGlobal("document", { documentElement: { dataset: {} as Record<string, string> } });

import {
  APP_THEMES,
  applyTheme,
  getCurrentMode,
  getCurrentTheme,
  getStoredTheme,
  setStoredTheme,
  subscribeTheme,
} from "./theme";

describe("theme", () => {
  beforeEach(() => {
    store.clear();
    (document.documentElement.dataset as Record<string, string>).theme = "";
  });

  it("未存 / 非法值回退 minimal", () => {
    expect(getStoredTheme()).toBe("minimal");
    store.set("hindsight.theme", "neon");
    expect(getStoredTheme()).toBe("minimal");
  });
  it.each(APP_THEMES)("%s 主题保存后可恢复且与当前主题一致", (theme) => {
    setStoredTheme(theme);
    expect(store.get("hindsight.theme")).toBe(theme);
    expect(getStoredTheme()).toBe(theme);
    expect(getCurrentTheme()).toBe(theme);

    // Simulate reopening the app by restoring only the persisted preference.
    document.documentElement.dataset.theme = "";
    applyTheme(getStoredTheme());
    expect(getCurrentTheme()).toBe(theme);
  });
  it("flat 是可选主题，切换时通知订阅者", () => {
    expect(APP_THEMES).toContain("flat");
    const onChange = vi.fn(() => getCurrentTheme());
    const off = subscribeTheme(onChange);
    setStoredTheme("flat");
    expect(onChange).toHaveReturnedWith("flat");
    off();
  });
  it("setStoredTheme 持久化 + 应用到 dataset + 通知订阅者", () => {
    const seen: string[] = [];
    const off = subscribeTheme(() => seen.push(getCurrentTheme()));
    setStoredTheme("dark");
    expect(store.get("hindsight.theme")).toBe("dark");
    expect(getCurrentTheme()).toBe("dark");
    expect(seen).toEqual(["dark"]);
    off();
    setStoredTheme("default");
    expect(seen).toEqual(["dark"]); // 退订后不再通知
  });
  it("applyTheme 只改 dataset;getCurrentTheme 对非法值回退 minimal", () => {
    applyTheme("default");
    expect(getCurrentTheme()).toBe("default");
    (document.documentElement.dataset as Record<string, string>).theme = "junk";
    expect(getCurrentTheme()).toBe("minimal");
  });
  it.each([
    ["default", "card", "light"],
    ["minimal", "card", "light"],
    ["flat", "flat", "light"],
    ["dark", "card", "dark"],
  ] as const)("内置主题 %s 设 data-layout=%s、data-mode=%s", (theme, layout, mode) => {
    applyTheme(theme);
    const { dataset } = document.documentElement;
    expect(dataset.theme).toBe(theme);
    expect(dataset.layout).toBe(layout);
    expect(dataset.mode).toBe(mode);
    expect(getCurrentMode()).toBe(mode);
  });
  it("getCurrentMode 只看 data-mode，没设或非法值当亮色", () => {
    const dataset = document.documentElement.dataset as Record<string, string>;
    dataset.theme = "dark";
    dataset.mode = "";
    expect(getCurrentMode()).toBe("light");
    dataset.mode = "dim";
    expect(getCurrentMode()).toBe("light");
    dataset.theme = "minimal";
    dataset.mode = "dark";
    expect(getCurrentMode()).toBe("dark");
  });
});
