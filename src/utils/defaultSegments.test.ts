import { describe, expect, it } from "vitest";
import { isDefaultSegments, retranslateDefaultSegments } from "./defaultSegments";
import type { AiSegment } from "../api/hindsight";

function seg(label: string, s: number, e: number, color = "#111111"): AiSegment {
  return { label, startHour: s, endHour: e, color };
}
const ZH: AiSegment[] = [
  seg("深夜", 0, 6),
  seg("早上", 6, 9),
  seg("上午", 9, 12),
  seg("下午", 12, 18),
  seg("晚上", 18, 24),
];

describe("isDefaultSegments", () => {
  it("简中默认整组命中 → true(颜色不参与判断)", () => {
    expect(isDefaultSegments(ZH)).toBe(true);
  });
  it("英文默认同样命中(跨语言默认集识别)", () => {
    const en = ["Late Night", "Early Morning", "Morning", "Afternoon", "Evening"].map(
      (l, i) => seg(l, ZH[i].startHour, ZH[i].endHour),
    );
    expect(isDefaultSegments(en)).toBe(true);
  });
  it("用户改过标签 → false,不得重译", () => {
    const edited = ZH.map((s, i) => (i === 2 ? { ...s, label: "干活" } : s));
    expect(isDefaultSegments(edited)).toBe(false);
  });
  it("用户改过区间 → false", () => {
    const edited = ZH.map((s, i) => (i === 0 ? { ...s, endHour: 7 } : s));
    expect(isDefaultSegments(edited)).toBe(false);
  });
  it("增删段 → false", () => {
    expect(isDefaultSegments(ZH.slice(0, 4))).toBe(false);
  });
});

describe("retranslateDefaultSegments", () => {
  it("zh → en:只换 label,区间与颜色保留", () => {
    const out = retranslateDefaultSegments(ZH, "en-US");
    expect(out.map((s) => s.label)).toEqual([
      "Late Night",
      "Early Morning",
      "Morning",
      "Afternoon",
      "Evening",
    ]);
    expect(out[0].startHour).toBe(0);
    expect(out[0].color).toBe("#111111");
  });
  it("未知语言回退英文;zh 家族归并到 zh-CN", () => {
    expect(retranslateDefaultSegments(ZH, "fr")[0].label).toBe("Late Night");
    expect(retranslateDefaultSegments(ZH, "zh-TW")[0].label).toBe("深夜");
  });
});
