import { describe, expect, it } from "vitest";
import { canMakeOwnGroup, pickPairingDrop, sameDrop, type Band } from "./pairingDrop";

// 两行，上下相邻；放置区在最后一行下面
const ROWS: [string, Band][] = [
  ["chrome", { top: 0, bottom: 40 }],
  ["slack", { top: 44, bottom: 84 }],
];
const ZONE: Band = { top: 92, bottom: 140 };

describe("pickPairingDrop", () => {
  it("drops on the group row under the cursor", () => {
    expect(pickPairingDrop(60, ROWS, ZONE)).toEqual({ kind: "group", groupId: "slack" });
  });

  it("makes a new group when dropped on the zone below the last row", () => {
    expect(pickPairingDrop(120, ROWS, ZONE)).toEqual({ kind: "newGroup" });
  });

  it("drops nowhere below the rows while the zone is not shown", () => {
    expect(pickPairingDrop(120, ROWS, null)).toBeNull();
  });

  it("drops nowhere in the gap between rows", () => {
    expect(pickPairingDrop(42, ROWS, ZONE)).toBeNull();
  });
});

describe("canMakeOwnGroup", () => {
  it("does not let the app a group is named after leave for a new group", () => {
    // 组 id 就是它的进程名：它已经在自己的组里
    expect(canMakeOwnGroup("chrome", "chrome")).toBe(false);
  });

  it("lets another member of the group get a group of its own", () => {
    expect(canMakeOwnGroup("chrome.exe", "chrome")).toBe(true);
  });
});

describe("sameDrop", () => {
  it("tells drops apart by kind and group", () => {
    expect(sameDrop({ kind: "newGroup" }, { kind: "newGroup" })).toBe(true);
    expect(sameDrop({ kind: "group", groupId: "a" }, { kind: "group", groupId: "a" })).toBe(true);
    expect(sameDrop({ kind: "group", groupId: "a" }, { kind: "group", groupId: "b" })).toBe(false);
    expect(sameDrop({ kind: "group", groupId: "a" }, { kind: "newGroup" })).toBe(false);
    expect(sameDrop(null, null)).toBe(true);
    expect(sameDrop(null, { kind: "newGroup" })).toBe(false);
  });
});
