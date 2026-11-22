import { describe, expect, it, vi } from "vitest";

vi.mock("@tauri-apps/api/core", () => ({
  invoke: () => Promise.resolve(null),
}));

import type { AiConfig, CloudConnection } from "../api/hindsight";
import {
  encodeModelRef,
  patchAdvanced,
  reportModelValue,
  reportRoute,
  reportThinking,
  reportThinkingModes,
  withReportThinking,
} from "./reportModel";

function conn(patch: Partial<CloudConnection> = {}): CloudConnection {
  return {
    id: "c1",
    name: "OpenAI",
    dialect: "openai",
    endpoint: "https://api.openai.com/v1",
    model: "gpt-4o-mini",
    apiKey: "sk-1",
    contextLength: null,
    ...patch,
  };
}

function ai(patch: Partial<AiConfig> = {}): AiConfig {
  return {
    cloud: [],
    picks: {
      daily: null,
      weekly: { kind: "inherit" },
      chat: null,
    },
    advanced: [],
    ...patch,
  } as AiConfig;
}

describe("reportRoute", () => {
  it("日报选了本地文件就用它", () => {
    const cfg = ai({
      picks: { daily: { kind: "local", file: "a.gguf" }, weekly: { kind: "inherit" }, chat: null },
    });
    expect(reportRoute(cfg, "daily")).toEqual({ cloud: null, local: "a.gguf" });
  });

  it("选了可用的云端连接走云端；不存在或未填齐则视为未选", () => {
    const c = conn();
    const ok = ai({
      cloud: [c],
      picks: {
        daily: { kind: "cloud", connection: "c1" },
        weekly: { kind: "inherit" },
        chat: null,
      },
    });
    expect(reportRoute(ok, "daily")).toEqual({ cloud: c, local: "" });

    const missing = ai({
      cloud: [],
      picks: {
        daily: { kind: "cloud", connection: "c1" },
        weekly: { kind: "inherit" },
        chat: null,
      },
    });
    expect(reportRoute(missing, "daily")).toEqual({ cloud: null, local: "" });
  });

  it("周报 inherit 时跟日报一样；单独选了就不看日报", () => {
    const c = conn();
    const base = ai({
      cloud: [c],
      picks: {
        daily: { kind: "cloud", connection: "c1" },
        weekly: { kind: "inherit" },
        chat: null,
      },
    });
    expect(reportRoute(base, "weekly")).toEqual(reportRoute(base, "daily"));

    const weeklyLocal = ai({
      cloud: [c],
      picks: {
        daily: { kind: "cloud", connection: "c1" },
        weekly: { kind: "local", file: "w.gguf" },
        chat: null,
      },
    });
    expect(reportRoute(weeklyLocal, "weekly")).toEqual({ cloud: null, local: "w.gguf" });
    expect(reportModelValue(weeklyLocal, "weekly")).toBe(
      encodeModelRef({ kind: "local", file: "w.gguf" }),
    );
  });
});

describe("reportThinking", () => {
  it("没设过是 auto；按「报告 + 实际用的模型」找", () => {
    const localA = { kind: "local" as const, file: "a.gguf" };
    const base = ai({
      picks: { daily: localA, weekly: { kind: "inherit" }, chat: null },
    });
    expect(reportThinking(base, "daily")).toBe("auto");

    const set = {
      ...base,
      advanced: [
        {
          task: "daily" as const,
          model: localA,
          reasoning: "off" as const,
          ctxSize: null,
          parallelSlots: null,
          batchSize: null,
        },
        {
          task: "weekly" as const,
          model: localA,
          reasoning: "high" as const,
          ctxSize: null,
          parallelSlots: null,
          batchSize: null,
        },
      ],
    };
    expect(reportThinking(set, "daily")).toBe("off");
    expect(reportThinking(set, "weekly")).toBe("high");

    const moved = {
      ...set,
      picks: { ...set.picks, weekly: { kind: "local" as const, file: "w.gguf" } },
    };
    expect(reportThinking(moved, "weekly")).toBe("auto");
  });

  it("改档位换掉这一项；改回 auto 且无其它覆盖时删掉整项", () => {
    const localA = { kind: "local" as const, file: "a.gguf" };
    const base = ai({ picks: { daily: localA, weekly: { kind: "inherit" }, chat: null } });
    const once = { ...base, advanced: withReportThinking(base, "daily", "low") };
    const twice = withReportThinking(once, "daily", "max");
    expect(twice).toEqual([
      {
        task: "daily",
        model: localA,
        reasoning: "max",
        ctxSize: null,
        parallelSlots: null,
        batchSize: null,
      },
    ]);
    expect(withReportThinking({ ...base, advanced: twice }, "daily", "auto")).toEqual([]);

    // 有其它覆盖（如 ctxSize）时，改回 auto 保留其它字段
    const withCtx = { ...base, advanced: patchAdvanced(once, "daily", localA, { ctxSize: 16384 }) };
    expect(withReportThinking(withCtx, "daily", "auto")).toEqual([
      {
        task: "daily",
        model: localA,
        reasoning: null,
        ctxSize: 16384,
        parallelSlots: null,
        batchSize: null,
      },
    ]);
  });
});

describe("reportThinkingModes", () => {
  it("档位跟着报告实际用的服务商方言走，不发工具", () => {
    const cfg = ai({
      cloud: [conn({ id: "c1", dialect: "openai" })],
      picks: {
        daily: { kind: "local", file: "a.gguf" },
        weekly: { kind: "cloud", connection: "c1" },
        chat: null,
      },
    });
    expect(reportThinkingModes(cfg, "daily")).toEqual(["auto", "high", "off"]);
    expect(reportThinkingModes(cfg, "weekly")).toContain("medium");
  });
});
