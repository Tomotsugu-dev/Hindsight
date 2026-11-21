import { describe, expect, it, vi } from "vitest";

vi.mock("@tauri-apps/api/core", () => ({
  invoke: () => Promise.resolve(null),
}));

import type { AiConfig, CloudConnection } from "../../api/hindsight";
import {
  chatCloudConnection,
  chatCloudReady,
  chatLocalModelName,
  chatUsesCloud,
} from "./chatRouting";

function conn(patch: Partial<CloudConnection> = {}): CloudConnection {
  return {
    id: "c1",
    name: "OpenAI",
    dialect: "openai",
    endpoint: "https://api.example.com/v1",
    model: "gpt-4o-mini",
    apiKey: "",
    contextLength: null,
    ...patch,
  };
}

function baseAi(patch: Partial<AiConfig> = {}): AiConfig {
  return {
    cloud: [],
    picks: {
      daily: null,
      weekly: { kind: "inherit" },
      chat: null,
    },
    advanced: [],
    modelsPath: "/models",
    userBrief: "",
    segments: [],
    visibility: {
      daily: { profiles: {}, autoProfile: null },
      weekly: { profiles: {}, autoProfile: null },
      chat: { profiles: {}, autoProfile: null },
    },
    promptLanguage: "zh",
    promptOverrides: {
      systemZh: "",
      systemEn: "",
      systemJa: "",
      systemPt: "",
      systemTw: "",
      systemEs: "",
    },
    autoSummary: false,
    autoSummaryTimes: [],
    ...patch,
  };
}

describe("chatCloudReady", () => {
  it("有填齐 endpoint + model 的云端连接时为 true（apiKey 可空）", () => {
    expect(chatCloudReady(baseAi({ cloud: [conn({ apiKey: "" })] }))).toBe(true);
  });

  it("缺 endpoint 或 model 时为 false", () => {
    expect(chatCloudReady(baseAi())).toBe(false);
    expect(chatCloudReady(baseAi({ cloud: [conn({ endpoint: "   " })] }))).toBe(false);
    expect(chatCloudReady(baseAi({ cloud: [conn({ model: "\t" })] }))).toBe(false);
  });
});

describe("chatUsesCloud & chatCloudConnection", () => {
  it("picks.chat 指向可用云端连接时走云端", () => {
    const ai = baseAi({
      cloud: [conn()],
      picks: {
        daily: null,
        weekly: { kind: "inherit" },
        chat: { kind: "cloud", connection: "c1" },
      },
    });
    expect(chatUsesCloud(ai)).toBe(true);
    expect(chatCloudConnection(ai)?.model).toBe("gpt-4o-mini");
  });

  it("picks.chat 指向不存在或未填齐的连接时不走云端", () => {
    const missing = baseAi({
      cloud: [conn()],
      picks: {
        daily: null,
        weekly: { kind: "inherit" },
        chat: { kind: "cloud", connection: "c9" },
      },
    });
    expect(chatUsesCloud(missing)).toBe(false);

    const incomplete = baseAi({
      cloud: [conn({ model: "" })],
      picks: {
        daily: null,
        weekly: { kind: "inherit" },
        chat: { kind: "cloud", connection: "c1" },
      },
    });
    expect(chatUsesCloud(incomplete)).toBe(false);
  });

  it("picks.chat 选本地或未选时不走云端", () => {
    const local = baseAi({
      cloud: [conn()],
      picks: { daily: null, weekly: { kind: "inherit" }, chat: { kind: "local", file: "a.gguf" } },
    });
    expect(chatUsesCloud(local)).toBe(false);
    expect(chatUsesCloud(baseAi({ cloud: [conn()] }))).toBe(false);
  });
});

describe("chatLocalModelName", () => {
  it("选本地返回 trim 后的文件名，否则空串", () => {
    const local = baseAi({
      picks: {
        daily: null,
        weekly: { kind: "inherit" },
        chat: { kind: "local", file: "  chat.gguf  " },
      },
    });
    expect(chatLocalModelName(local)).toBe("chat.gguf");
    expect(chatLocalModelName(baseAi())).toBe("");
  });
});
