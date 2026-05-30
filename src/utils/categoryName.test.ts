import { describe, it, expect } from "vitest";
import type { TFunction } from "i18next";
import { displayCategoryName, displaySuperCategoryName } from "./categoryName";

// 桩 t：直接回显 key，方便断言「走没走 i18n」。
const t = ((key: string) => key) as unknown as TFunction;

describe("displayCategoryName", () => {
  it("默认分类且未改名 → 走 i18n（返回 categories.defaults.<id> key）", () => {
    expect(displayCategoryName({ id: "code", name: "编程" }, t)).toBe(
      "categories.defaults.code",
    );
  });

  it("默认分类但被用户改过名 → 用 category.name，不翻译", () => {
    expect(displayCategoryName({ id: "code", name: "我的编程" }, t)).toBe("我的编程");
  });

  it("非默认分类（id 不在映射表）→ 直接用 name", () => {
    expect(displayCategoryName({ id: "custom-123", name: "自定义" }, t)).toBe("自定义");
  });
});

describe("displaySuperCategoryName", () => {
  it("默认大类且未改名 → 走 i18n", () => {
    expect(displaySuperCategoryName({ id: "work", name: "工作" }, t)).toBe(
      "categories.super.defaults.work",
    );
  });

  it("默认大类被改名 → 用 sup.name", () => {
    expect(displaySuperCategoryName({ id: "work", name: "搬砖" }, t)).toBe("搬砖");
  });

  it("非默认大类 → 直接用 name", () => {
    expect(displaySuperCategoryName({ id: "misc", name: "杂项" }, t)).toBe("杂项");
  });
});
