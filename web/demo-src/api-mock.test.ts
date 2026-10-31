import { expect, it, vi } from "vitest";
import { api as real } from "@app/api/hindsight";

// 首页的演示把 api 整个换成 mock。少一个方法，页面在 effect 里调用时直接抛 TypeError，整个演示白屏
it("has exactly the methods of the real API", async () => {
  // api-mock 一加载就读语言和保存的状态，测试环境没有 localStorage
  vi.stubGlobal("localStorage", { getItem: () => null, setItem: () => {} });
  const { api: mock } = await import("./api-mock");

  expect(Object.keys(mock).sort()).toEqual(Object.keys(real).sort());
});
