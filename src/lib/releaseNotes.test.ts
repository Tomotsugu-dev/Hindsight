import { describe, expect, it } from "vitest";
import { pickReleaseNotesForLang } from "./releaseNotes";

const BODY = `<!-- zh -->
【0.7.7】
- 中文改动
<!-- en -->
【0.7.7】
- English change
<!-- pt -->
【0.7.7】
- Mudança em português`;

describe("pickReleaseNotesForLang", () => {
  it("picks the block matching the locale prefix", () => {
    expect(pickReleaseNotesForLang(BODY, "pt-BR")).toBe("【0.7.7】\n- Mudança em português");
    expect(pickReleaseNotesForLang(BODY, "zh-CN")).toBe("【0.7.7】\n- 中文改动");
    expect(pickReleaseNotesForLang(BODY, "en")).toBe("【0.7.7】\n- English change");
  });

  it("falls back to English when the locale has no block", () => {
    expect(pickReleaseNotesForLang(BODY, "ja")).toBe("【0.7.7】\n- English change");
  });

  it("maps Traditional Chinese locales to the tw block", () => {
    const withTw = `${BODY}\n<!-- tw -->\n【0.7.7】\n- 繁體變更`;
    expect(pickReleaseNotesForLang(withTw, "zh-TW")).toBe("【0.7.7】\n- 繁體變更");
    expect(pickReleaseNotesForLang(withTw, "zh-Hant-HK")).toBe("【0.7.7】\n- 繁體變更");
    // 繁体块缺失 → 回退简体而不是英文
    expect(pickReleaseNotesForLang(BODY, "zh-TW")).toBe("【0.7.7】\n- 中文改动");
    // 简体用户不受 tw 块影响
    expect(pickReleaseNotesForLang(withTw, "zh-CN")).toBe("【0.7.7】\n- 中文改动");
  });

  it("returns the whole body when there are no markers (old releases)", () => {
    const plain = "【0.7.6】\n- bilingual stuff";
    expect(pickReleaseNotesForLang(plain, "pt-BR")).toBe(plain);
  });

  it("falls back to the whole body when neither the locale nor en is present", () => {
    const zhOnly = "<!-- zh -->\n【x】\n- 只有中文";
    expect(pickReleaseNotesForLang(zhOnly, "fr")).toBe("【x】\n- 只有中文");
  });

  it("keeps the markdown intact so the dialog can render it", () => {
    // 更新弹窗把这个返回值交给 MarkdownText 渲染，所以列表符号必须原样留着。
    // 早期实现把整块插进一条 i18n 文案当纯文本显示，"-" 就变成了字面短横线。
    const picked = pickReleaseNotesForLang(BODY, "en");
    expect(picked.split("\n")).toContain("- English change");
  });

  it("maps Spanish locales to the es block", () => {
    const withEs = `${BODY}\n<!-- es -->\n【0.7.7】\n- Cambio en español`;
    expect(pickReleaseNotesForLang(withEs, "es")).toBe("【0.7.7】\n- Cambio en español");
    expect(pickReleaseNotesForLang(withEs, "es-MX")).toBe("【0.7.7】\n- Cambio en español");
    // 版本说明还没写 es 块时回退英文，而不是整坨多语言全塞给用户
    expect(pickReleaseNotesForLang(BODY, "es")).toBe("【0.7.7】\n- English change");
  });
});
