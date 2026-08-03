import { describe, expect, it } from "vitest";
import type { ExternalProfile } from "../api/hindsight";
import { profileLabel } from "./profileLabel";

function p(patch: Partial<ExternalProfile>): ExternalProfile {
  return {
    name: "",
    provider: "custom",
    endpoint: "https://example.com/v1",
    apiKey: "",
    model: "",
    ...patch,
  };
}

describe("profileLabel", () => {
  it("模型名已带厂商时不再前缀 provider（避免「kimi · kimi-k2.7-code」的复读）", () => {
    expect(profileLabel(p({ provider: "kimi", model: "kimi-k2.7-code" }))).toBe(
      "kimi-k2.7-code",
    );
    expect(
      profileLabel(p({ provider: "deepseek", model: "deepseek-v4-flash" })),
    ).toBe("deepseek-v4-flash");
  });

  it("模型名看不出出处时补上 provider", () => {
    expect(profileLabel(p({ provider: "openai", model: "gpt-5.6-luna" }))).toBe(
      "openai · gpt-5.6-luna",
    );
  });

  it("kimi 与 kimi-cn 必须可区分——用完整 provider 判前缀，不取首段", () => {
    // 国内站的模型名不以 "kimi-cn" 开头，于是保留前缀，两条配置不会撞脸
    expect(
      profileLabel(p({ provider: "kimi-cn", model: "kimi-k2.7-code" })),
    ).toBe("kimi-cn · kimi-k2.7-code");
  });

  it("大小写不影响判定", () => {
    expect(profileLabel(p({ provider: "DeepSeek", model: "deepseek-chat" }))).toBe(
      "deepseek-chat",
    );
  });

  it("没填模型时退回域名，域名解析不了才用原始 endpoint", () => {
    expect(profileLabel(p({ provider: "openai", model: "" }))).toBe("example.com");
    expect(profileLabel(p({ model: "", endpoint: "半截地址" }))).toBe("半截地址");
  });
});
