import { expect, it, vi } from "vitest";
import { api as real } from "@app/api/hindsight";

// 首页的演示把 api 整个换成 mock。少一个方法，页面在 effect 里调用时直接抛 TypeError，整个演示白屏
it("has exactly the methods of the real API", async () => {
  // api-mock 一加载就读语言和保存的状态，测试环境没有 localStorage
  vi.stubGlobal("localStorage", { getItem: () => null, setItem: () => {} });
  const { api: mock } = await import("./api-mock");

  expect(Object.keys(mock).sort()).toEqual(Object.keys(real).sort());
});

// 归类应用页的拖动在演示里要和真的应用一样：一个应用只在一个组里
it("moves a merged app out of its old group", async () => {
  vi.stubGlobal("localStorage", { getItem: () => null, setItem: () => {} });
  const { api: mock } = await import("./api-mock");

  await mock.mergeAppGroup("cursor.exe", "g-terminal");
  const holders = (await mock.listAppGroups()).filter((g) =>
    g.members.some((m) => m.processName === "cursor.exe"),
  );
  expect(holders.map((g) => g.id)).toEqual(["g-terminal"]);
});

it("puts an unmerged app in a group of its own, with the category of the group it leaves", async () => {
  vi.stubGlobal("localStorage", { getItem: () => null, setItem: () => {} });
  const { api: mock } = await import("./api-mock");

  await mock.unmergeAppGroup("Code");
  const groups = await mock.listAppGroups();
  const own = groups.find((g) => g.id === "Code");
  expect(own?.categoryId).toBe("code");
  expect(own?.members.map((m) => m.processName)).toEqual(["Code"]);
  const vscode = groups.find((g) => g.id === "g-vscode");
  expect(vscode?.members.some((m) => m.processName === "Code")).toBe(false);
});
