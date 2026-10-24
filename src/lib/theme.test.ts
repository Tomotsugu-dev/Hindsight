// node 环境手工桩:theme.ts 只用到 localStorage 与 documentElement 的 dataset、style,
// 不必为此挂 jsdom(vitest 配置保持纯函数策略)。
import { beforeEach, describe, expect, it, vi } from "vitest";
import { installFakeThemeDom } from "./fakeThemeDom";

const { store, tokens, dataset, sheets } = installFakeThemeDom();

import {
  APP_THEMES,
  applyTheme,
  customChoice,
  customIdOf,
  getCurrentMode,
  getCurrentTheme,
  getStoredCustomTheme,
  getStoredTheme,
  restoreTheme,
  setCustomTheme,
  setStoredTheme,
  showFallbackTheme,
  subscribeTheme,
  type CustomTheme,
} from "./theme";

const ocean: CustomTheme = {
  id: "ocean",
  layout: "flat",
  mode: "dark",
  tokens: { "--color-accent": "#0af", "--radius-md": "2px" },
  css: "[data-part='card'] { outline: 1px solid red; }",
};

describe("theme", () => {
  beforeEach(() => {
    applyTheme("minimal"); // 撤掉上一个测试套用的自定义主题
    store.clear();
    for (const key of Object.keys(dataset)) delete dataset[key];
    dataset.theme = "";
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
    restoreTheme();
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
    dataset.theme = "junk";
    expect(getCurrentTheme()).toBe("minimal");
  });
  it.each([
    ["default", "card", "light"],
    ["minimal", "card", "light"],
    ["flat", "flat", "light"],
    ["dark", "card", "dark"],
  ] as const)("内置主题 %s 设 data-layout=%s、data-mode=%s", (theme, layout, mode) => {
    applyTheme(theme);
    expect(dataset.theme).toBe(theme);
    expect(dataset.layout).toBe(layout);
    expect(dataset.mode).toBe(mode);
    expect(getCurrentMode()).toBe(mode);
  });
  it("getCurrentMode 只看 data-mode，没设或非法值当亮色", () => {
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

describe("自定义主题", () => {
  beforeEach(() => {
    applyTheme("minimal");
    store.clear();
  });

  it("custom:<id> 存得住；只有前缀没有 id 时回退 minimal", () => {
    store.set("hindsight.theme", "custom:ocean");
    expect(getStoredTheme()).toBe("custom:ocean");
    expect(customIdOf(getStoredTheme())).toBe("ocean");
    store.set("hindsight.theme", "custom:");
    expect(getStoredTheme()).toBe("minimal");
    expect(customIdOf("dark")).toBeNull();
    expect(customChoice("ocean")).toBe("custom:ocean");
  });

  it("setCustomTheme 存选择和副本，写三个属性、变量和 theme.css，通知订阅者", () => {
    const onChange = vi.fn();
    const off = subscribeTheme(onChange);
    setCustomTheme(ocean);
    off();

    expect(store.get("hindsight.theme")).toBe("custom:ocean");
    expect(getStoredCustomTheme()).toEqual(ocean);
    expect(dataset.theme).toBe("custom");
    expect(dataset.layout).toBe("flat");
    expect(dataset.mode).toBe("dark");
    expect(getCurrentMode()).toBe("dark");
    expect(Object.fromEntries(tokens)).toEqual(ocean.tokens);
    expect(sheets()).toEqual([ocean.css]);
    expect(onChange).toHaveBeenCalledTimes(1);
  });

  it("换回内置主题时撤掉变量、theme.css 和副本", () => {
    setCustomTheme(ocean);
    setStoredTheme("default");
    expect(tokens.size).toBe(0);
    expect(sheets()).toEqual([]);
    expect(store.has("hindsight.theme.custom")).toBe(false);
    expect(dataset.theme).toBe("default");
  });

  it("换成另一个自定义主题时，前一个主题的变量和 theme.css 都不留", () => {
    setCustomTheme(ocean);
    setCustomTheme({ ...ocean, id: "plain", tokens: { "--text-strong": "#111" }, css: null });
    expect(Object.fromEntries(tokens)).toEqual({ "--text-strong": "#111" });
    expect(sheets()).toEqual([]);
  });

  it("restoreTheme：副本就是选的主题时直接套用", () => {
    setCustomTheme(ocean);
    applyTheme("minimal"); // 模拟重新打开窗口：页面上什么都没有
    restoreTheme();
    expect(dataset.theme).toBe("custom");
    expect(Object.fromEntries(tokens)).toEqual(ocean.tokens);
    expect(sheets()).toEqual([ocean.css]);
  });

  it("restoreTheme：副本是别的主题时先显示简约，删掉副本", () => {
    store.set("hindsight.theme", "custom:forest");
    store.set("hindsight.theme.custom", JSON.stringify(ocean));
    restoreTheme();
    expect(dataset.theme).toBe("minimal");
    expect(tokens.size).toBe(0);
    expect(store.has("hindsight.theme.custom")).toBe(false);
    expect(getStoredTheme()).toBe("custom:forest");
  });

  it.each([
    ["不是 JSON", "{"],
    ["mode 不认识", JSON.stringify({ ...ocean, mode: "dim" })],
    ["变量值不是字符串", JSON.stringify({ ...ocean, tokens: { "--color-accent": 1 } })],
    ["缺 css", JSON.stringify({ ...ocean, css: undefined })],
  ])("副本格式不对（%s）时当没有副本", (_, raw) => {
    store.set("hindsight.theme", "custom:ocean");
    store.set("hindsight.theme.custom", raw);
    expect(getStoredCustomTheme()).toBeNull();
    restoreTheme();
    expect(dataset.theme).toBe("minimal");
  });

  it("showFallbackTheme 保留选择、删掉副本、显示简约；已经是简约时不再通知", () => {
    setCustomTheme(ocean);
    const onChange = vi.fn();
    const off = subscribeTheme(onChange);
    showFallbackTheme();
    showFallbackTheme();
    off();

    expect(getStoredTheme()).toBe("custom:ocean");
    expect(store.has("hindsight.theme.custom")).toBe(false);
    expect(dataset.theme).toBe("minimal");
    expect(tokens.size).toBe(0);
    expect(sheets()).toEqual([]);
    expect(onChange).toHaveBeenCalledTimes(1);
  });

  it("副本存不下时照样套用，但不留副本", () => {
    store.set("hindsight.theme.custom", JSON.stringify({ ...ocean, id: "old" }));
    const setItem = vi.spyOn(localStorage, "setItem").mockImplementation((k, v) => {
      if (k === "hindsight.theme.custom") throw new Error("QuotaExceededError");
      store.set(k, v);
    });
    try {
      setCustomTheme(ocean);
    } finally {
      setItem.mockRestore();
    }
    expect(dataset.theme).toBe("custom");
    expect(getStoredTheme()).toBe("custom:ocean");
    expect(store.has("hindsight.theme.custom")).toBe(false);
  });
});
