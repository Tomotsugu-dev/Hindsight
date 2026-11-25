import { expect, it, vi } from "vitest";

/** 换上一份浏览器里的存档，重新加载演示的假后端，就像访客刷新了页面。 */
async function reload(saved: string | null) {
  const written: string[] = [];
  vi.stubGlobal("localStorage", {
    getItem: () => saved,
    setItem: (_key: string, value: string) => written.push(value),
    removeItem: () => {},
  });
  vi.resetModules();
  const { api } = await import("./api-mock");
  return { api, written };
}

// 旧版演示存的设置缺后来加的字段（比如 ai.picks），对话页一读就崩。
// 数据结构不一样的存档整份不用，回到默认值
it("ignores a save made when the demo's data had another shape", async () => {
  const { mockSettings } = await import("./fixtures");
  const oldAi: Partial<typeof mockSettings.ai> = { ...mockSettings.ai };
  delete oldAi.picks;
  const { api } = await reload(JSON.stringify({ settings: { ...mockSettings, ai: oldAi } }));

  expect((await api.getSettings()).ai.picks).toEqual(mockSettings.ai.picks);
});

// 同一版演示存的改动，刷新后还在
it("keeps a save made by the same demo", async () => {
  const first = await reload(null);
  await first.api.updateSettings({ captureEnabled: false });
  const { api } = await reload(first.written[first.written.length - 1]);

  expect((await api.getSettings()).captureEnabled).toBe(false);
});
