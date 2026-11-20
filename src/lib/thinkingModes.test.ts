import { describe, expect, it } from "vitest";
import { normalizeThinking, thinkingLabelKey, thinkingModes } from "./thinkingModes";

describe("thinkingModes", () => {
  it("openai 发工具时只能关；不发工具（报告）时有强度", () => {
    expect(thinkingModes("openai", true)).toEqual(["off"]);
    expect(thinkingModes("openai", false)).toEqual(["auto", "low", "medium", "high", "off"]);
  });

  it("其它服务商发不发工具都一样", () => {
    expect(thinkingModes("deepseek", false)).toEqual(thinkingModes("deepseek", true));
    expect(thinkingModes("kimi", false)).toEqual(["auto", "low", "medium", "high", "off"]);
  });

  it("本地和不认识的服务商只有开关", () => {
    expect(thinkingModes(null, false)).toEqual(["auto", "high", "off"]);
    expect(thinkingModes("custom", true)).toEqual(["auto", "high", "off"]);
  });
});

describe("normalizeThinking", () => {
  it("表里有就原样用", () => {
    expect(normalizeThinking("max", thinkingModes("deepseek", false))).toBe("max");
  });

  it("强度类不在表里就近取 high", () => {
    expect(normalizeThinking("max", thinkingModes("kimi", false))).toBe("high");
    expect(normalizeThinking("medium", thinkingModes(null, false))).toBe("high");
  });

  it("其余取 auto；连 auto 都没有就取第一档", () => {
    expect(normalizeThinking("weird", thinkingModes(null, false))).toBe("auto");
    expect(normalizeThinking("high", thinkingModes("openai", true))).toBe("off");
  });
});

describe("thinkingLabelKey", () => {
  it("只有开关时 high 叫「开启」，有强度时叫「高」", () => {
    expect(thinkingLabelKey("high", thinkingModes(null, false))).toBe("chat.thinkingMode.on");
    expect(thinkingLabelKey("high", thinkingModes("kimi", false))).toBe("chat.thinkingMode.high");
  });
});
