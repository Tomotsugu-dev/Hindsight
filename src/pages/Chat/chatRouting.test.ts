import { describe, expect, it, vi } from "vitest";

// chatRouting 经 ../../api/hindsight 间接 import @tauri-apps/api/core;
// node 环境没有 Tauri IPC,把 invoke 桩掉(本文件只测纯函数,不会真调)。
vi.mock("@tauri-apps/api/core", () => ({
  invoke: () => Promise.resolve(null),
}));

import { SUMMARY_CLOUD_SENTINEL, type AiConfig } from "../../api/hindsight";
import {
  chatCloudReady,
  chatLocalModelName,
  chatUsesCloud,
} from "./chatRouting";

// ⚠️ 镜像契约:本文件测的三个函数是后端 src-tauri/src/ai/config.rs 里
// chat_cloud_ready / chat_use_cloud / effective_chat_main(+effective_summary_main
// fallback 链)的逐字前端镜像。**两边改任意一处,另一边和这份测试必须同步改**,
// 否则 ModelBadge 显示的路由和后端实际路由会分叉(UI 说走云端、实际跑本地)。

/** 造一个字段齐全的 AiConfig;测试只覆盖关心的字段。 */
function baseAi(patch: Partial<AiConfig> = {}): AiConfig {
  return {
    endpoint: "",
    model: "",
    apiKey: "",
    externalEnabled: false,
    externalProvider: "custom",
    userBrief: "",
    segments: [],
    excludedCategories: [],
    modelsPath: "/models",
    activeMain: "",
    activeMmproj: "",
    summaryMain: "",
    summaryMmproj: "",
    chatMain: "",
    promptLanguage: "zh",
    promptOverrides: {
      systemZh: "",
      systemEn: "",
      systemJa: "",
      systemPt: "",
      systemTw: "",
    },
    batchSize: null,
    parallelSlots: null,
    ctxSize: null,
    autoSummary: false,
  autoSummaryAt: null,
  autoSummaryTimes: [],
    summaryBatchSize: null,
    summaryParallelSlots: null,
    summaryCtxSize: null,
    ...patch,
  };
}

/** 云端三元组齐备的底座(apiKey 故意留空——它不在必要集里)。 */
function cloudReadyAi(patch: Partial<AiConfig> = {}): AiConfig {
  return baseAi({
    externalEnabled: true,
    endpoint: "https://api.example.com/v1",
    model: "gpt-4o-mini",
    ...patch,
  });
}

describe("chatCloudReady(镜像 config.rs chat_cloud_ready)", () => {
  it("enabled + endpoint + model 三元组齐 → true;apiKey 不参与判定", () => {
    // 部分自建 OpenAI 兼容端点无鉴权,所以 apiKey 为空也算 ready
    expect(chatCloudReady(cloudReadyAi({ apiKey: "" }))).toBe(true);
  });

  it("三元组缺一不可", () => {
    expect(chatCloudReady(cloudReadyAi({ externalEnabled: false }))).toBe(false);
    expect(chatCloudReady(cloudReadyAi({ endpoint: "" }))).toBe(false);
    expect(chatCloudReady(cloudReadyAi({ model: "" }))).toBe(false);
  });

  it("纯空白 endpoint / model 等同缺失(trim 后判空,同后端语义)", () => {
    expect(chatCloudReady(cloudReadyAi({ endpoint: "   " }))).toBe(false);
    expect(chatCloudReady(cloudReadyAi({ model: "\t " }))).toBe(false);
  });
});

describe("chatUsesCloud(镜像 config.rs chat_use_cloud)", () => {
  it("显式本地文件名 → 永远本地,哪怕云端三元组配齐", () => {
    expect(chatUsesCloud(cloudReadyAi({ chatMain: "local.gguf" }))).toBe(false);
  });

  it("显式 sentinel → 云端 ready 才上云;不 ready 退化本地不硬卡", () => {
    expect(chatUsesCloud(cloudReadyAi({ chatMain: SUMMARY_CLOUD_SENTINEL }))).toBe(
      true,
    );
    // sentinel 残留但三元组不齐:不能报错卡死,静默走本地 fallback
    expect(
      chatUsesCloud(cloudReadyAi({ chatMain: SUMMARY_CLOUD_SENTINEL, model: "" })),
    ).toBe(false);
    expect(
      chatUsesCloud(baseAi({ chatMain: SUMMARY_CLOUD_SENTINEL })),
    ).toBe(false);
  });

  it("空(自动)→ 跟随云端 ready 状态", () => {
    expect(chatUsesCloud(cloudReadyAi({ chatMain: "" }))).toBe(true);
    expect(
      chatUsesCloud(cloudReadyAi({ chatMain: "", externalEnabled: false })),
    ).toBe(false);
  });

  it("纯空白 chatMain 视同空(自动),不当成本地文件名", () => {
    expect(chatUsesCloud(cloudReadyAi({ chatMain: "   " }))).toBe(true);
  });
});

describe("chatLocalModelName(镜像 config.rs effective_chat_main 三级链)", () => {
  it("一级:chatMain 显式文件名优先,summary/active 全部忽略", () => {
    const ai = baseAi({
      chatMain: "chat.gguf",
      summaryMain: "sum.gguf",
      activeMain: "active.gguf",
    });
    expect(chatLocalModelName(ai)).toBe("chat.gguf");
  });

  it("二级:chatMain 空或 sentinel → 穿透到 summaryMain", () => {
    const base = { summaryMain: "sum.gguf", activeMain: "active.gguf" };
    expect(chatLocalModelName(baseAi({ ...base, chatMain: "" }))).toBe("sum.gguf");
    expect(
      chatLocalModelName(baseAi({ ...base, chatMain: SUMMARY_CLOUD_SENTINEL })),
    ).toBe("sum.gguf");
  });

  it("三级:summaryMain 也空或为 sentinel → 落到 activeMain", () => {
    // sentinel 是"用云端"标记不是文件名,本地路径必须穿过它看 activeMain,
    // 否则会把 "__cloud__" 当文件名去加载
    expect(
      chatLocalModelName(
        baseAi({ chatMain: "", summaryMain: "", activeMain: "active.gguf" }),
      ),
    ).toBe("active.gguf");
    expect(
      chatLocalModelName(
        baseAi({
          chatMain: SUMMARY_CLOUD_SENTINEL,
          summaryMain: SUMMARY_CLOUD_SENTINEL,
          activeMain: "active.gguf",
        }),
      ),
    ).toBe("active.gguf");
  });

  it("三级全空 → 空串(调用方据此显示「需要一个语言模型」)", () => {
    expect(chatLocalModelName(baseAi())).toBe("");
  });

  it("返回值已 trim(显式名带空白时不会带进引擎启动参数)", () => {
    expect(chatLocalModelName(baseAi({ chatMain: "  chat.gguf  " }))).toBe(
      "chat.gguf",
    );
    expect(chatLocalModelName(baseAi({ activeMain: " active.gguf " }))).toBe(
      "active.gguf",
    );
  });
});
