import { describe, expect, it } from "vitest";
import type { CloudConnection } from "../api/hindsight";
import { profileLabel } from "./profileLabel";

function p(patch: Partial<CloudConnection>): CloudConnection {
  return {
    id: "c1",
    name: "",
    dialect: "custom",
    endpoint: "https://example.com/v1",
    apiKey: "",
    model: "",
    contextLength: null,
    ...patch,
  };
}

describe("profileLabel", () => {
  it("模型名已带厂商时不再前缀 dialect", () => {
    expect(profileLabel(p({ dialect: "kimi", model: "kimi-k2.7-code" }))).toBe("kimi-k2.7-code");
    expect(profileLabel(p({ dialect: "deepseek", model: "deepseek-v4-flash" }))).toBe(
      "deepseek-v4-flash",
    );
  });

  it("模型名看不出出处时补上 dialect", () => {
    expect(profileLabel(p({ dialect: "openai", model: "gpt-5.6-luna" }))).toBe(
      "openai · gpt-5.6-luna",
    );
  });

  it("kimi 与 kimi-cn 必须可区分", () => {
    expect(profileLabel(p({ dialect: "kimi-cn", model: "kimi-k2.7-code" }))).toBe(
      "kimi-cn · kimi-k2.7-code",
    );
  });

  it("大小写不影响判定", () => {
    expect(profileLabel(p({ dialect: "DeepSeek", model: "deepseek-chat" }))).toBe("deepseek-chat");
  });

  it("没填模型时退回域名，域名解析不了才用原始 endpoint", () => {
    expect(profileLabel(p({ dialect: "openai", model: "" }))).toBe("example.com");
    expect(profileLabel(p({ model: "", endpoint: "半截地址" }))).toBe("半截地址");
  });
});
