import { describe, expect, it } from "vitest";
import { thumbGeometry } from "./thumbGeometry";

// Issue 2A:主窗口/下拉菜单的自绘滚动条几何。
describe("thumbGeometry", () => {
  it("内容装得下时返回 null(不显示滚动条)", () => {
    expect(thumbGeometry(0, 500, 500)).toBeNull();
    expect(thumbGeometry(0, 300, 500)).toBeNull();
    // 1px 误差容忍(亚像素高度常见)
    expect(thumbGeometry(0, 501, 500)).toBeNull();
  });

  it("顶部时 topPct=0,高度按可视比例", () => {
    const g = thumbGeometry(0, 1000, 500)!;
    expect(g.topPct).toBe(0);
    expect(g.heightPct).toBe(50);
  });

  it("滚到底时 thumb 底边贴轨道底(top + height = 100)", () => {
    const g = thumbGeometry(500, 1000, 500)!;
    expect(g.topPct + g.heightPct).toBeCloseTo(100);
  });

  it("中点滚动位置映射到轨道中段", () => {
    const g = thumbGeometry(250, 1000, 500)!;
    expect(g.topPct).toBeCloseTo(25);
  });

  it("超长内容 thumb 有最小高度,不缩成米粒", () => {
    const g = thumbGeometry(0, 100000, 500)!;
    expect(g.heightPct).toBe(8);
  });

  it("rubber-band 越界滚动被钳住,不飞出轨道", () => {
    const over = thumbGeometry(600, 1000, 500)!;
    expect(over.topPct + over.heightPct).toBeLessThanOrEqual(100);
    const under = thumbGeometry(-50, 1000, 500)!;
    expect(under.topPct).toBe(0);
  });
});
