import { describe, it, expect } from "vitest";
import {
  isLightHex,
  resolveSegmentChip,
  resolveSegmentDotColor,
} from "./segmentColor";

describe("isLightHex", () => {
  it("纯白判为浅色", () => {
    expect(isLightHex("#ffffff")).toBe(true);
  });

  it("纯黑判为深色", () => {
    expect(isLightHex("#000000")).toBe(false);
  });

  it("大小写不敏感", () => {
    expect(isLightHex("#FFFFFF")).toBe(true);
  });

  it("非法 hex（短格式 / 非颜色）回退为浅色 true", () => {
    expect(isLightHex("#fff")).toBe(true);
    expect(isLightHex("not-a-color")).toBe(true);
    expect(isLightHex("")).toBe(true);
  });

  it("按 perceived luminance 区分明暗：亮黄=浅，深蓝=深", () => {
    expect(isLightHex("#ffe066")).toBe(true);
    expect(isLightHex("#1d1c25")).toBe(false);
  });
});

describe("resolveSegmentChip", () => {
  it("配了自定义颜色时原样返回 background，并按 hex 判明暗", () => {
    const chip = resolveSegmentChip({ startHour: 9, endHour: 10, color: "#000000" });
    expect(chip.background).toBe("#000000");
    expect(chip.isLight).toBe(false);
  });

  it("自定义浅色 → isLight=true", () => {
    const chip = resolveSegmentChip({ startHour: 9, endHour: 10, color: "#ffffff" });
    expect(chip.background).toBe("#ffffff");
    expect(chip.isLight).toBe(true);
  });

  it("未配颜色（空串）→ 按段中点自动 HSL 渐变，返回 hsl(...) 字符串", () => {
    const chip = resolveSegmentChip({ startHour: 9, endHour: 11, color: "" });
    expect(chip.background).toMatch(/^hsl\(\d+, \d+%, \d+%\)$/);
  });

  it("未配颜色（纯空白）也走自动渐变，而非当成自定义色", () => {
    const chip = resolveSegmentChip({ startHour: 0, endHour: 1, color: "   " });
    expect(chip.background).toMatch(/^hsl\(/);
  });

  it("自动渐变在浅色段返回 isLight=true（L>60）", () => {
    // 中午前后是高亮度暖色（L≈82），应判为浅色
    const chip = resolveSegmentChip({ startHour: 10, endHour: 11, color: "" });
    expect(chip.isLight).toBe(true);
  });

  it("同一输入稳定（确定性，无随机）", () => {
    const a = resolveSegmentChip({ startHour: 14, endHour: 15, color: "" });
    const b = resolveSegmentChip({ startHour: 14, endHour: 15, color: "" });
    expect(a).toEqual(b);
  });
});

describe("resolveSegmentDotColor", () => {
  it("用户配了 hex 原样返回", () => {
    expect(
      resolveSegmentDotColor({ startHour: 9, endHour: 12, color: "#ff8800" }),
    ).toBe("#ff8800");
  });

  it("自动色压暗到 l=58%（粉彩底色调直接当 8px 色点会看不清）", () => {
    const c = resolveSegmentDotColor({ startHour: 10, endHour: 11, color: "" });
    expect(c).toMatch(/^hsl\(\d+, \d+%, 58%\)$/);
  });

  it("同一输入稳定（确定性，无随机）", () => {
    const a = resolveSegmentDotColor({ startHour: 14, endHour: 15, color: "" });
    const b = resolveSegmentDotColor({ startHour: 14, endHour: 15, color: "" });
    expect(a).toBe(b);
  });
});
