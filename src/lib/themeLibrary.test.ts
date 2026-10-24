import { beforeEach, describe, expect, it, vi } from "vitest";
import { installFakeThemeDom } from "./fakeThemeDom";
import type { InstalledTheme, Theme } from "../api/themes";

const { store, tokens, dataset, sheets } = installFakeThemeDom();

const api = vi.hoisted(() => ({
  list: vi.fn<() => Promise<InstalledTheme[]>>(),
  css: vi.fn<(id: string) => Promise<string | null>>(),
}));
vi.mock("../api/themes", () => ({ themesApi: api }));
vi.mock("./logger", () => ({ logError: vi.fn() }));

import { chooseCustomTheme, loadThemes } from "./themeLibrary";
import { applyTheme, getStoredTheme, setStoredTheme, subscribeTheme } from "./theme";

const ocean: Theme = {
  id: "ocean",
  name: "Ocean",
  version: "1.0.0",
  author: "",
  description: {},
  layout: "card",
  mode: "dark",
  tokens: { "--color-accent": "#0af" },
  ignoredTokens: [],
  hasCss: true,
};
const OCEAN_CSS = "[data-part='card'] { border-color: #0af; }";

const ok = (theme: Theme): InstalledTheme => ({ folder: theme.id, theme, error: null });

describe("themeLibrary", () => {
  beforeEach(() => {
    applyTheme("minimal");
    store.clear();
    api.list.mockReset();
    api.css.mockReset();
    api.css.mockResolvedValue(OCEAN_CSS);
  });

  it("选的是内置主题时只列主题，不读 CSS、不换主题", async () => {
    setStoredTheme("dark");
    api.list.mockResolvedValue([ok(ocean)]);
    expect(await loadThemes()).toEqual([ok(ocean)]);
    expect(api.css).not.toHaveBeenCalled();
    expect(dataset.theme).toBe("dark");
  });

  it("chooseCustomTheme 读 theme.css 后套用", async () => {
    await chooseCustomTheme(ocean);
    expect(api.css).toHaveBeenCalledWith("ocean");
    expect(getStoredTheme()).toBe("custom:ocean");
    expect(dataset.theme).toBe("custom");
    expect(sheets()).toEqual([OCEAN_CSS]);
  });

  it("没有 theme.css 的主题不去读", async () => {
    await chooseCustomTheme({ ...ocean, hasCss: false });
    expect(api.css).not.toHaveBeenCalled();
    expect(sheets()).toEqual([]);
  });

  it("主题文件改了：读主题库时换成新内容", async () => {
    await chooseCustomTheme(ocean);
    api.list.mockResolvedValue([ok({ ...ocean, tokens: { "--color-accent": "#f00" } })]);
    api.css.mockResolvedValue(null);
    await loadThemes();
    expect(tokens.get("--color-accent")).toBe("#f00");
    expect(sheets()).toEqual([]);
  });

  it("主题文件没变：不重新套用，不通知", async () => {
    await chooseCustomTheme(ocean);
    api.list.mockResolvedValue([ok(ocean)]);
    const onChange = vi.fn();
    const off = subscribeTheme(onChange);
    await loadThemes();
    off();
    expect(onChange).not.toHaveBeenCalled();
  });

  it.each([
    ["主题库里没有了", [] as InstalledTheme[]],
    ["主题坏了", [{ folder: "ocean", theme: null, error: "theme: no theme.json" }]],
  ])("%s：保留选择，先显示简约", async (_, list) => {
    await chooseCustomTheme(ocean);
    api.list.mockResolvedValue(list);
    await loadThemes();
    expect(getStoredTheme()).toBe("custom:ocean");
    expect(dataset.theme).toBe("minimal");
    expect(tokens.size).toBe(0);
    expect(store.has("hindsight.theme.custom")).toBe(false);
  });

  it("主题修好后自动换回来", async () => {
    await chooseCustomTheme(ocean);
    api.list.mockResolvedValue([]);
    await loadThemes();
    api.list.mockResolvedValue([ok(ocean)]);
    await loadThemes();
    expect(dataset.theme).toBe("custom");
    expect(sheets()).toEqual([OCEAN_CSS]);
  });

  it("列出来以后 theme.css 读不出来：当作主题坏了", async () => {
    await chooseCustomTheme(ocean);
    api.list.mockResolvedValue([ok({ ...ocean, tokens: {} })]);
    api.css.mockRejectedValue("theme: theme.css is not UTF-8");
    await loadThemes();
    expect(dataset.theme).toBe("minimal");
    expect(getStoredTheme()).toBe("custom:ocean");
  });

  it("等 theme.css 的时候用户换了主题：不再套用读到的主题", async () => {
    await chooseCustomTheme({ ...ocean, hasCss: false });
    let release: (css: string) => void = () => {};
    api.css.mockReturnValue(new Promise((resolve) => (release = resolve)));
    api.list.mockResolvedValue([ok({ ...ocean, tokens: { "--color-accent": "#f00" } })]);
    const loading = loadThemes();
    await vi.waitFor(() => expect(api.css).toHaveBeenCalled());
    setStoredTheme("flat");
    release(OCEAN_CSS);
    await loading;
    expect(dataset.theme).toBe("flat");
    expect(tokens.size).toBe(0);
    expect(sheets()).toEqual([]);
  });
});
