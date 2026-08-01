import { describe, expect, it } from "vitest";
import {
  clampHour,
  formatHour,
  hourFromRatio,
  nearestIndex,
  nextFreeHour,
  parseHour,
} from "./timeOfDayMath";

// TimeOfDayPicker 的纯数学(定时计划两条 24h 时间轴共用)。
describe("timeOfDayMath", () => {
  it("hourFromRatio:每格覆盖 [h/24,(h+1)/24),两端钳位", () => {
    expect(hourFromRatio(0)).toBe(0);
    expect(hourFromRatio(0.5 / 24)).toBe(0);
    expect(hourFromRatio(1 / 24)).toBe(1);
    expect(hourFromRatio(23.5 / 24)).toBe(23);
    expect(hourFromRatio(0.9999)).toBe(23);
    // 拖出轨道左右:钳到边界格
    expect(hourFromRatio(-0.3)).toBe(0);
    expect(hourFromRatio(1.7)).toBe(23);
    // 恰好 1.0(指针停在最右缘):floor 给 24,必须钳回 23
    expect(hourFromRatio(1)).toBe(23);
  });

  it("formatHour:两位补零 + :00", () => {
    expect(formatHour(0)).toBe("00:00");
    expect(formatHour(9)).toBe("09:00");
    expect(formatHour(23)).toBe("23:00");
  });

  it("parseHour:取小时位、忽略分钟、坏输入回落 0 并钳位", () => {
    expect(parseHour("23:00")).toBe(23);
    expect(parseHour("03:45")).toBe(3);
    expect(parseHour("31:00")).toBe(23);
    expect(parseHour("")).toBe(0);
    expect(parseHour("abc")).toBe(0);
  });

  it("nearestIndex:取最近标记,平手取先添加的", () => {
    expect(nearestIndex([23], 5)).toBe(0);
    expect(nearestIndex([9, 21], 10)).toBe(0);
    expect(nearestIndex([9, 21], 20)).toBe(1);
    // 平手(距 15 各差 3):取下标小的(先添加)
    expect(nearestIndex([12, 18], 15)).toBe(0);
    expect(nearestIndex([], 5)).toBe(0);
  });

  it("nextFreeHour:从 12 点顺时针找空位;避开已占;全占回落 12", () => {
    expect(nextFreeHour([])).toBe(12);
    expect(nextFreeHour([23])).toBe(12);
    expect(nextFreeHour([12])).toBe(13);
    expect(nextFreeHour([12, 13, 14])).toBe(15);
    // 跨午夜回绕
    const all_but_11 = Array.from({ length: 24 }, (_, i) => i).filter((h) => h !== 11);
    expect(nextFreeHour(all_but_11)).toBe(11);
    const all = Array.from({ length: 24 }, (_, i) => i);
    expect(nextFreeHour(all)).toBe(12);
  });

  it("clampHour:0..23", () => {
    expect(clampHour(-5)).toBe(0);
    expect(clampHour(24)).toBe(23);
    expect(clampHour(12)).toBe(12);
  });
});
