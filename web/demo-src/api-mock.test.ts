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

// 周报页收到 all_done 才停止转圈；收到 segment_done 后按周一再取一次正文
it("finishes a weekly report and keeps it for that week", async () => {
  vi.stubGlobal("localStorage", { getItem: () => null, setItem: () => {} });
  vi.useFakeTimers();
  const { api: mock, SUMMARY_PROGRESS_EVENT } = await import("./api-mock");
  const { listen } = await import("./tauri/event");
  const phases: string[] = [];
  const unlisten = await listen<{ phase: string; source: string }>(SUMMARY_PROGRESS_EVENT, (e) => {
    if (e.payload.source === "weekly") phases.push(e.payload.phase);
  });

  const run = mock.generateWeekSummary("2026-11-23", false, true);
  await vi.runAllTimersAsync();
  await run;
  unlisten();
  vi.useRealTimers();

  expect(phases[phases.length - 1]).toBe("all_done");
  const row = await mock.getWeekSummary("2026-11-23");
  expect(row).toMatchObject({
    source: "weekly",
    localDate: "2026-11-23",
    label: "2026-11-23 ~ 2026-11-29",
    status: "ok",
  });
  expect(row?.content).not.toBe("");

  await mock.clearWeekSummary("2026-11-23");
  expect(await mock.getWeekSummary("2026-11-23")).toBeNull();
});
