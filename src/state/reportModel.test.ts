import { describe, expect, it, vi } from "vitest";

// reportModel 经 ../api/hindsight 间接 import @tauri-apps/api/core；本文件只测纯函数，桩掉 invoke
vi.mock("@tauri-apps/api/core", () => ({
  invoke: () => Promise.resolve(null),
}));

import { SUMMARY_CLOUD_SENTINEL, type AiConfig } from "../api/hindsight";
import {
  reportModelValue,
  reportRoute,
  reportThinking,
  reportThinkingModes,
  withReportThinking,
} from "./reportModel";

// reportRoute 照后端 AiConfig::task_model 算，两边改一处另一边要跟着改

/** 只填这些函数读的字段 */
function ai(patch: Partial<AiConfig> = {}): AiConfig {
  return {
    summaryMain: "",
    activeMain: "",
    externalEnabled: false,
    externalProvider: "custom",
    ...patch,
  } as AiConfig;
}

describe("reportRoute", () => {
  it("日报选了本地文件就用它", () => {
    expect(reportRoute(ai({ summaryMain: "a.gguf", activeMain: "b.gguf" }), "daily")).toEqual({
      cloud: false,
      local: "a.gguf",
    });
  });

  it("日报没选时用 activeMain", () => {
    expect(reportRoute(ai({ activeMain: "b.gguf" }), "daily")).toEqual({
      cloud: false,
      local: "b.gguf",
    });
  });

  it("选了云端但云端关着，退回本地", () => {
    const off = ai({ summaryMain: SUMMARY_CLOUD_SENTINEL, activeMain: "b.gguf" });
    expect(reportRoute(off, "daily")).toEqual({ cloud: false, local: "b.gguf" });
    expect(reportRoute({ ...off, externalEnabled: true }, "daily").cloud).toBe(true);
  });

  it("周报 inherit（含旧后端不返回 picks）时跟日报一样", () => {
    const base = ai({ summaryMain: SUMMARY_CLOUD_SENTINEL, externalEnabled: true });
    expect(reportRoute(base, "weekly")).toEqual(reportRoute(base, "daily"));
    const inherit = { ...base, picks: { weekly: { kind: "inherit" as const } } };
    expect(reportRoute(inherit, "weekly").cloud).toBe(true);
  });

  it("周报单独选了就不看日报", () => {
    const base = ai({ summaryMain: SUMMARY_CLOUD_SENTINEL, externalEnabled: true });
    const local = { ...base, picks: { weekly: { kind: "local" as const, file: "w.gguf" } } };
    expect(reportRoute(local, "weekly")).toEqual({ cloud: false, local: "w.gguf" });
    expect(reportModelValue(local, "weekly")).toBe("w.gguf");
  });
});

describe("reportThinking", () => {
  it("没设过是 auto；按「报告 + 实际用的模型」找", () => {
    const base = ai({ summaryMain: "a.gguf" });
    expect(reportThinking(base, "daily")).toBe("auto");
    const set = {
      ...base,
      advanced: [
        {
          task: "daily" as const,
          model: { kind: "local" as const, file: "a.gguf" },
          reasoning: "off" as const,
        },
        {
          task: "weekly" as const,
          model: { kind: "local" as const, file: "a.gguf" },
          reasoning: "high" as const,
        },
      ],
    };
    expect(reportThinking(set, "daily")).toBe("off");
    // 周报 inherit，实际用的是日报的模型
    expect(reportThinking(set, "weekly")).toBe("high");
    // 周报换了模型，这个模型没设过
    const moved = { ...set, picks: { weekly: { kind: "local" as const, file: "w.gguf" } } };
    expect(reportThinking(moved, "weekly")).toBe("auto");
  });

  it("改档位换掉这一项；改回 auto 就删掉它", () => {
    const base = ai({ summaryMain: "a.gguf" });
    const once = { ...base, advanced: withReportThinking(base, "daily", "low") };
    const twice = withReportThinking(once, "daily", "max");
    expect(twice).toEqual([
      { task: "daily", model: { kind: "local", file: "a.gguf" }, reasoning: "max" },
    ]);
    expect(withReportThinking({ ...base, advanced: twice }, "daily", "auto")).toEqual([]);
  });
});

describe("reportThinkingModes", () => {
  it("档位跟着报告实际用的服务商，不发工具", () => {
    const openai = ai({ externalProvider: "openai", externalEnabled: true });
    // 日报走本地：只有开关
    expect(reportThinkingModes(openai, "daily")).toEqual(["auto", "high", "off"]);
    // 周报走云端 openai：报告不发工具，有强度
    const weeklyCloud = { ...openai, picks: { weekly: { kind: "cloud" as const } } };
    expect(reportThinkingModes(weeklyCloud, "weekly")).toContain("medium");
  });
});
