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

  it("returns the whole body when there are no markers (old releases)", () => {
    const plain = "【0.7.6】\n- bilingual stuff";
    expect(pickReleaseNotesForLang(plain, "pt-BR")).toBe(plain);
  });

  it("falls back to the whole body when neither the locale nor en is present", () => {
    const zhOnly = "<!-- zh -->\n【x】\n- 只有中文";
    expect(pickReleaseNotesForLang(zhOnly, "fr")).toBe("【x】\n- 只有中文");
  });
});
