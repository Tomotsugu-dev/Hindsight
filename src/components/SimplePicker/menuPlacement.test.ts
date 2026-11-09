import { describe, expect, it } from "vitest";
import { menuSize, placeMenu } from "./menuPlacement";

// 视口 720 高，trigger 高 30，间距 6，菜单离窗口边缘留 8
const VIEWPORT = 720;
const GAP = 6;

describe("placeMenu", () => {
  it("下方装得下就向下展开", () => {
    const p = placeMenu({ top: 100, bottom: 130 }, VIEWPORT, 200, GAP);
    expect(p.direction).toBe("down");
    // 720 - 130 - 6 - 8
    expect(p.maxHeight).toBe(576);
  });

  it("下方装不下、上方空间更大时向上展开", () => {
    const p = placeMenu({ top: 600, bottom: 630 }, VIEWPORT, 200, GAP);
    expect(p.direction).toBe("up");
    // 600 - 6 - 8
    expect(p.maxHeight).toBe(586);
  });

  it("两边都装不下时往空间大的一边展开，并把高度限制在那一边的空间内", () => {
    // 插件版本很多的情况：菜单 650 高，上下都放不下
    const up = placeMenu({ top: 400, bottom: 430 }, VIEWPORT, 650, GAP);
    expect(up).toEqual({ direction: "up", maxHeight: 386 });

    const down = placeMenu({ top: 200, bottom: 230 }, VIEWPORT, 650, GAP);
    expect(down).toEqual({ direction: "down", maxHeight: 476 });
  });

  it("菜单高度正好等于下方空间时仍向下展开", () => {
    const p = placeMenu({ top: 500, bottom: 530 }, VIEWPORT, 176, GAP);
    expect(p).toEqual({ direction: "down", maxHeight: 176 });
  });
});

describe("menuSize", () => {
  it("默认密度：每项 32、上下内边距共 8、最多露 8 项", () => {
    expect(menuSize(3, 1)).toEqual({ height: 104, maxHeight: 264, gap: 6 });
  });

  it("主题把密度调小，估算的高度跟着变小，不按默认尺寸放菜单", () => {
    expect(menuSize(3, 0.5)).toEqual({ height: 52, maxHeight: 132, gap: 3 });
  });
});
