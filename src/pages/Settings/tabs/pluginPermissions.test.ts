import { describe, it, expect } from "vitest";
import { keptChoices, permissionTree, permissionsUnder, setChecked } from "./pluginPermissions";

// 后端 permission_depends_on 返回的一部分
const dependsOn: Record<string, string> = {
  "stats.today": "stats.read",
  "stats.month": "stats.read",
  "categories.app.read": "categories.read",
  "categories.app.write": "categories.app.read",
  "focus.app": "focus.changed",
  "focus.title": "focus.changed",
};

const asked = [
  "stats.read",
  "stats.today",
  "stats.month",
  "categories.read",
  "categories.app.read",
  "categories.app.write",
  "notifications",
  "focus.changed",
  "focus.app",
  "focus.title",
];

describe("permissionTree", () => {
  it("每个权限挂在它依赖的那个下面，同一层按清单的顺序", () => {
    const tree = permissionTree(asked, dependsOn);
    expect(tree.map((n) => n.name)).toEqual([
      "stats.read",
      "categories.read",
      "notifications",
      "focus.changed",
    ]);
    expect(tree[0].children.map((n) => n.name)).toEqual(["stats.today", "stats.month"]);
    expect(permissionsUnder(tree[1])).toEqual(["categories.app.read", "categories.app.write"]);
    expect(tree[2].children).toEqual([]);
  });
});

describe("setChecked", () => {
  it("取消一个，依赖它的也取消，包括下面的下面", () => {
    expect(setChecked(asked, "categories.read", false, asked, dependsOn)).toEqual(
      asked.filter((p) => !p.startsWith("categories.")),
    );
  });

  it("勾上一个，它依赖的也勾上，别的不动", () => {
    expect(setChecked(["notifications"], "categories.app.write", true, asked, dependsOn)).toEqual([
      "categories.read",
      "categories.app.read",
      "categories.app.write",
      "notifications",
    ]);
  });

  it("取消最后一档时间范围，stats.read 也取消", () => {
    const checked = ["stats.read", "stats.month", "notifications"];
    expect(setChecked(checked, "stats.month", false, asked, dependsOn)).toEqual(["notifications"]);
  });

  it("勾上 stats.read 时一档都没有，每一档都勾上；勾上一档只多出 stats.read", () => {
    expect(setChecked([], "stats.read", true, asked, dependsOn)).toEqual([
      "stats.read",
      "stats.today",
      "stats.month",
    ]);
    expect(setChecked([], "stats.month", true, asked, dependsOn)).toEqual([
      "stats.read",
      "stats.month",
    ]);
  });
});

describe("keptChoices", () => {
  it("同意过的还勾着，拒绝过的还不勾，新申请的勾上，旧版本有新版本没有的去掉", () => {
    const oldAsked = ["notifications", "focus.changed", "focus.app", "focus.title", "island"];
    const oldApproved = ["notifications", "focus.changed", "focus.app", "island"];
    expect(keptChoices(asked, oldAsked, oldApproved, dependsOn)).toEqual(
      asked.filter((p) => p !== "focus.title"),
    );
  });

  it("新申请的权限依赖的那个以前拒绝了，新的也不勾", () => {
    const oldAsked = ["focus.changed", "focus.app"];
    expect(keptChoices(asked, oldAsked, [], dependsOn)).toEqual(
      asked.filter((p) => !p.startsWith("focus.")),
    );
  });

  it("同意过的那档新版本不要了，剩下的都拒绝过，stats.read 也不勾", () => {
    const newAsked = ["stats.read", "stats.month"];
    const oldAsked = ["stats.read", "stats.today", "stats.month"];
    expect(keptChoices(newAsked, oldAsked, ["stats.read", "stats.today"], dependsOn)).toEqual([]);
  });
});
