import { describe, it, expect } from "vitest";
import { ignoreKeywordFromTitle } from "./ignoreKeyword";

describe("ignoreKeywordFromTitle", () => {
  it("spinner 前缀的各种变体提炼出同一个关键词", () => {
    // Claude Code 转圈动画每帧换字符——同一任务产生 ⠐/✳/⠂ 多种标题,
    // 关键词必须收敛到同一个,否则规则只命中 1/3 的行
    const base = "Download videos from July 17 onwards with uv";
    for (const s of ["⠐ ", "✳ ", "⠂ ", ""]) {
      expect(ignoreKeywordFromTitle(`${s}${base}`)).toBe(base);
    }
  });

  it("普通标题原样保留,结尾括号是内容不剥", () => {
    expect(ignoreKeywordFromTitle("会议纪要(2)")).toBe("会议纪要(2)");
    expect(ignoreKeywordFromTitle("vim main.rs")).toBe("vim main.rs");
  });

  it("首尾空白与结尾 spinner 一并剥掉", () => {
    expect(ignoreKeywordFromTitle("  vim main.rs ⠐ ")).toBe("vim main.rs");
  });

  it("纯符号标题剥完为空时回退原串", () => {
    expect(ignoreKeywordFromTitle("⠐ ⠂")).toBe("⠐ ⠂");
  });

  it("空标题返回空串(调用方据此隐藏按钮)", () => {
    expect(ignoreKeywordFromTitle("   ")).toBe("");
  });
});
