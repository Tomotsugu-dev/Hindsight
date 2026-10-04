import { describe, it, expect } from "vitest";
import {
  fitSegments,
  offsetToShow,
  OTHERS_ID,
  ringWindow,
  type RingItem,
  type RingSegment,
} from "./ringItems";

const item = (id: string, minutes: number): RingItem => ({
  id,
  name: id,
  color: "#000",
  minutes,
  superId: "s",
});

const six = [
  item("a", 50),
  item("b", 40),
  item("c", 30),
  item("d", 20),
  item("e", 10),
  item("f", 5),
];

describe("ringWindow", () => {
  it("不超过圈数时全部露出来，按分钟数从多到少，滑不动", () => {
    const w = ringWindow([item("a", 10), item("b", 30), item("c", 20)], 4, 2);
    expect(w.visible.map((r) => r.id)).toEqual(["b", "c", "a"]);
    expect(w.offset).toBe(0);
  });

  it("往下滑一格：最外圈那名退出，最内圈补进下一名", () => {
    expect(ringWindow(six, 4, 0).visible.map((r) => r.id)).toEqual(["a", "b", "c", "d"]);
    expect(ringWindow(six, 4, 1).visible.map((r) => r.id)).toEqual(["b", "c", "d", "e"]);
  });

  it("滑过头时夹到能放满的最后一格，不会露出空圈", () => {
    const w = ringWindow(six, 4, 9);
    expect(w.offset).toBe(2);
    expect(w.visible.map((r) => r.id)).toEqual(["c", "d", "e", "f"]);
    expect(w.total).toBe(6);
    expect(ringWindow(six, 4, -3).offset).toBe(0);
  });

  it("没有时间的小类不算", () => {
    expect(ringWindow([item("a", 0), item("b", 5)], 4, 0).visible.map((r) => r.id)).toEqual([
      "b",
    ]);
  });
});

describe("offsetToShow", () => {
  it("已经露着时不挪", () => {
    expect(offsetToShow(six, 4, 1, "c")).toBe(1);
  });

  it("在窗口下面：挪到它在最内圈", () => {
    // 窗口露 a–d，点了第 6 名 f → 露 c–f
    expect(offsetToShow(six, 4, 0, "f")).toBe(2);
  });

  it("在窗口上面：挪到它在最外圈", () => {
    expect(offsetToShow(six, 4, 2, "a")).toBe(0);
  });

  it("找不到时不挪", () => {
    expect(offsetToShow(six, 4, 1, "zzz")).toBe(1);
  });
});

const seg =(id: string, minutes: number): RingSegment => ({
  id,
  name: id,
  color: "#000",
  minutes,
  merged: 0,
});

describe("fitSegments", () => {
  it("都够大时原样按分钟数从多到少，比例就是真实比例", () => {
    const { segments, fractions } = fitSegments([seg("a", 30), seg("b", 70)], 0.03, "#999");
    expect(segments.map((s) => s.id)).toEqual(["b", "a"]);
    expect(fractions).toEqual([0.7, 0.3]);
  });

  it("两段及以上不够大时合成「其他」，分钟数不丢", () => {
    const { segments } = fitSegments(
      [seg("a", 900), seg("b", 80), seg("c", 10), seg("d", 10)],
      0.03,
      "#999",
    );
    expect(segments.map((s) => s.id)).toEqual(["a", "b", OTHERS_ID]);
    expect(segments[2]).toMatchObject({ minutes: 20, merged: 2, color: "#999" });
  });

  it("只有一段不够大时保留它自己，画的时候抬到最小比例，从最大那段扣", () => {
    // 1106 分钟里工作沟通只有 3 分钟（0.27%）
    const { segments, fractions } = fitSegments([seg("code", 1103), seg("chat", 3)], 0.03, "#999");
    expect(segments.map((s) => s.id)).toEqual(["code", "chat"]);
    expect(fractions[1]).toBeCloseTo(0.03);
    expect(fractions[0] + fractions[1]).toBeCloseTo(1);
  });

  it("只有一段时占满整圈", () => {
    expect(fitSegments([seg("a", 5)], 0.03, "#999").fractions).toEqual([1]);
  });
});
