import { beforeAll, describe, expect, it } from "vitest";
import i18next from "i18next";
import { localizeAiError } from "./aiErrors";

beforeAll(async () => {
  await i18next.init({
    lng: "zh",
    resources: {
      zh: { translation: { aiSummary: { errors: { llmEmptyEos: "模型空答复" } } } },
    },
  });
});

describe("localizeAiError", () => {
  it("命中已知错误码 → 本地化文案", () => {
    expect(localizeAiError("[LLM_EMPTY_EOS] eos hit with no content")).toBe("模型空答复");
  });
  it("码前带外层包装前缀也能命中(正则不锚定开头)", () => {
    expect(localizeAiError("llm response: [LLM_EMPTY_EOS] xxx")).toBe("模型空答复");
  });
  it("未知码 / 无码原样返回,不丢信息", () => {
    expect(localizeAiError("[WHO_KNOWS] boom")).toBe("[WHO_KNOWS] boom");
    expect(localizeAiError("plain failure")).toBe("plain failure");
  });
});
