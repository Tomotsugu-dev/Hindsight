import { describe, expect, it } from "vitest";
import {
  ASSIGN_MENU_MAX_HEIGHT,
  assignMenuLayout,
} from "./menuLayout";

// Issue 1A 回归:分类多时菜单曾无限增高、超出视口的分类点不到。
// 纯函数层钉死:封顶、按侧收紧、翻转、top 永不越出视口。
describe("assignMenuLayout", () => {
  const trigger = { top: 100, bottom: 128, left: 40, width: 90 };

  it("空间充足时贴 trigger 下方,少量分类不撑到上限", () => {
    const r = assignMenuLayout({ trigger, viewportHeight: 900, optionCount: 4 });
    expect(r.top).toBe(trigger.bottom + 4);
    expect(r.left).toBe(40);
    expect(r.width).toBe(90);
    // 4 行远小于上限:maxHeight 只由可用空间/封顶决定,内容自身收缩由
    // ScrollBox fit="max" 负责,这里只需保证上限 ≥ 内容
    expect(r.maxHeight).toBe(ASSIGN_MENU_MAX_HEIGHT);
  });

  it("分类很多时封顶到 ASSIGN_MENU_MAX_HEIGHT(1A 主场景)", () => {
    const r = assignMenuLayout({ trigger, viewportHeight: 900, optionCount: 40 });
    expect(r.maxHeight).toBe(ASSIGN_MENU_MAX_HEIGHT);
    expect(r.top).toBe(trigger.bottom + 4);
  });

  it("下方放不下且上方更宽裕时翻到上方,菜单底边贴住 trigger", () => {
    const low = { top: 700, bottom: 728, left: 40, width: 90 };
    const r = assignMenuLayout({ trigger: low, viewportHeight: 811, optionCount: 40 });
    // 上方空间 692 > 下方 75 → 翻转;高度封顶 264
    expect(r.maxHeight).toBe(ASSIGN_MENU_MAX_HEIGHT);
    expect(r.top).toBe(700 - ASSIGN_MENU_MAX_HEIGHT - 4);
  });

  it("翻转且内容不足上限时按内容锚定,不留悬空缝隙", () => {
    const low = { top: 700, bottom: 728, left: 40, width: 90 };
    const r = assignMenuLayout({ trigger: low, viewportHeight: 811, optionCount: 5 });
    const contentEstimate = 5 * 27 + 10;
    expect(r.top).toBe(700 - contentEstimate - 4);
  });

  it("矮窗口里 maxHeight 收紧到所选侧的可用空间", () => {
    const t = { top: 60, bottom: 88, left: 40, width: 90 };
    const r = assignMenuLayout({ trigger: t, viewportHeight: 300, optionCount: 40 });
    // 下方空间 300-88-8=204 < 264 但 > 上方 52 → 不翻转,收紧到 204
    expect(r.maxHeight).toBe(204);
    expect(r.top).toBe(88 + 4);
  });

  it("翻转侧空间不足 264 时按该侧收紧,top 永不越出视口顶部", () => {
    const t = { top: 120, bottom: 148, left: 40, width: 90 };
    const r = assignMenuLayout({ trigger: t, viewportHeight: 170, optionCount: 40 });
    // 上方 112 > 下方 14 → 翻转;收紧到 112;top = max(8, 120-112-4) = 8
    expect(r.maxHeight).toBe(112);
    expect(r.top).toBe(8);
  });

  it("两侧都极小时走 96px 兜底(宁可轻微越界也不给不可用的窄条)", () => {
    const t = { top: 60, bottom: 88, left: 40, width: 90 };
    const r = assignMenuLayout({ trigger: t, viewportHeight: 150, optionCount: 40 });
    // 下方 54、上方 52 → 不翻转;min(264,54)=54 < 96 → 兜底 96
    expect(r.maxHeight).toBe(96);
    expect(r.top).toBe(88 + 4);
  });
});
