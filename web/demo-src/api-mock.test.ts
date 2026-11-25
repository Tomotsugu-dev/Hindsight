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

// 周报的每日检查和正文一样用页面的语言；星期名照应用里的表
it("names the days of the weekly check in the page language", async () => {
  vi.stubGlobal("localStorage", { getItem: () => null, setItem: () => {} });
  const { api: mock } = await import("./api-mock");
  const i18n = (await import("@app/i18n")).default;

  const names = async (lng: string) => {
    await i18n.changeLanguage(lng);
    return (await mock.precheckWeekSummary("2026-11-23")).days.map((d) => d.weekday);
  };
  expect(await names("en")).toEqual(["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"]);
  expect(await names("ja")).toEqual(["月", "火", "水", "木", "金", "土", "日"]);
  expect((await names("es"))[2]).toBe("Mié");
  expect((await names("pt"))[0]).toBe("Seg");
  expect((await names("zh-CN"))[6]).toBe("周日");
});

/** 跑完 run 期间的所有计时器，返回 source 这一路收到的最后一个阶段。 */
async function lastPhase(source: string, run: () => Promise<unknown>): Promise<string> {
  const { SUMMARY_PROGRESS_EVENT } = await import("./api-mock");
  const { listen } = await import("./tauri/event");
  const phases: string[] = [];
  const unlisten = await listen<{ phase: string; source: string }>(SUMMARY_PROGRESS_EVENT, (e) => {
    if (e.payload.source === source) phases.push(e.payload.phase);
  });
  const done = run();
  await vi.runAllTimersAsync();
  await done;
  unlisten();
  return phases[phases.length - 1];
}

// 停止要真的停：日报、周报共用一个停止开关，收尾发 cancelled，不再写后面的段
it("stops a daily or weekly report when asked", async () => {
  vi.stubGlobal("localStorage", { getItem: () => null, setItem: () => {} });
  vi.useFakeTimers();
  const { api: mock } = await import("./api-mock");

  const daily = await lastPhase("daily", async () => {
    const run = mock.generateDaySummary("2026-11-20", true, null);
    await mock.cancelDaySummary();
    await run;
  });
  const weekly = await lastPhase("weekly", async () => {
    const run = mock.generateWeekSummary("2026-11-16", false, true);
    await mock.cancelDaySummary();
    await run;
  });
  vi.useRealTimers();

  expect(daily).toBe("cancelled");
  expect(weekly).toBe("cancelled");
  expect(await mock.getWeekSummary("2026-11-16")).toBeNull();
});

// 和应用一样：停止后问题留在会话里，回答不存
it("keeps the question but not the answer when a chat is stopped", async () => {
  vi.stubGlobal("localStorage", { getItem: () => null, setItem: () => {} });
  vi.useFakeTimers();
  const { api: mock } = await import("./api-mock");

  const run = mock.chatAsk("stop me", null, "en", "ask-1");
  expect(await mock.chatCancel("ask-1")).toBe(true);
  await vi.runAllTimersAsync();
  const result = await run;
  vi.useRealTimers();

  expect(result.cancelled).toBe(true);
  const messages = await mock.chatGetMessages(result.conversationId);
  expect(messages.map((m) => m.role)).toEqual(["user"]);
  expect(await mock.chatCancel("ask-1")).toBe(false);
});

// 演示的默认设置存的是中文。英文页面保存一次设置（比如确认隐私）后，示例值仍是英文；
// 访客自己写的内容原样保留
it("keeps the sample AI settings in the page language after a save", async () => {
  vi.stubGlobal("localStorage", { getItem: () => null, setItem: () => {} });
  const { api: mock } = await import("./api-mock");
  const { DEMO_PROFILE_ID } = await import("./fixtures");
  const i18n = (await import("@app/i18n")).default;
  await i18n.changeLanguage("en");

  const shown = await mock.getSettings();
  const saved = await mock.updateSettings({ chatPrivacyAcknowledged: true });
  expect(saved.ai.userBrief).toBe(shown.ai.userBrief);
  expect(saved.ai.segments).toEqual(shown.ai.segments);
  expect(saved.ai.promptLanguage).toBe("en");
  expect(saved.ai.visibility?.profiles?.[DEMO_PROFILE_ID]?.name).toBe(
    shown.ai.visibility?.profiles?.[DEMO_PROFILE_ID]?.name,
  );

  const edited = await mock.updateSettings({
    ai: {
      ...saved.ai,
      userBrief: "My own words",
      promptOverrides: { ...saved.ai.promptOverrides, systemEn: "My prompt" },
    },
  });
  expect(edited.ai.userBrief).toBe("My own words");
  expect(edited.ai.promptOverrides.systemEn).toBe("My prompt");
  await i18n.changeLanguage("zh-CN");
});

// 改过的段一直在：切走再回来时页面会重新读，读到的还是改过的文字；重新生成后回到示例
it("keeps an edited report segment until the report runs again", async () => {
  vi.stubGlobal("localStorage", { getItem: () => null, setItem: () => {} });
  const { api: mock } = await import("./api-mock");
  const d = new Date();
  const today = `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}-${String(d.getDate()).padStart(2, "0")}`;

  const before = await mock.getDaySummary(today);
  await mock.editSummarySegment("daily", today, 1, "EDITMARK");
  const reread = await mock.getDaySummary(today);
  expect(reread[1].content).toBe("EDITMARK");
  expect(reread[0].content).toBe(before[0].content);

  vi.useFakeTimers();
  const run = mock.generateDaySummary(today, true, null);
  await vi.runAllTimersAsync();
  await run;
  vi.useRealTimers();
  expect((await mock.getDaySummary(today))[1].content).toBe(before[1].content);
});

// 下完的模型出现在「我的模型」里，删掉后消失
it("lists a downloaded model under My models until it is deleted", async () => {
  vi.stubGlobal("localStorage", { getItem: () => null, setItem: () => {} });
  vi.useFakeTimers();
  const { api: mock } = await import("./api-mock");

  const run = mock.downloadModel("demo/repo", "new-model.gguf", 1000);
  await vi.runAllTimersAsync();
  const path = await run;
  vi.useRealTimers();

  const listed = (await mock.listLocalModels()).find((m) => m.filename === "new-model.gguf");
  expect(listed).toMatchObject({ path, sizeBytes: 1000, isMmproj: false });
  await mock.deleteModel("new-model.gguf");
  expect((await mock.listLocalModels()).some((m) => m.filename === "new-model.gguf")).toBe(false);
});

// 暂停的下载留下半成品，界面据此显示「继续」；再下从断点接着下
it("pauses a model download and keeps what it got", async () => {
  vi.stubGlobal("localStorage", { getItem: () => null, setItem: () => {} });
  vi.useFakeTimers();
  const { api: mock } = await import("./api-mock");

  const run = mock.downloadModel("demo/repo", "paused.gguf", 2000);
  const failed = run.catch((e: unknown) => String(e));
  await vi.advanceTimersByTimeAsync(500);
  await mock.cancelModelDownload("paused.gguf");
  await vi.runAllTimersAsync();
  const error = await failed;

  expect(error).toMatch(/^download cancelled/);
  const partial = (await mock.listPartialDownloads()).find((p) => p.filename === "paused.gguf");
  expect(partial?.downloadedBytes).toBeGreaterThan(0);

  const resumed = mock.downloadModel("demo/repo", "paused.gguf", 2000);
  await vi.runAllTimersAsync();
  await resumed;
  vi.useRealTimers();
  expect((await mock.listPartialDownloads()).some((p) => p.filename === "paused.gguf")).toBe(false);
});
