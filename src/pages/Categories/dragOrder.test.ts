import { describe, expect, it } from "vitest";
import {
  insertSuperAfter,
  insertBlockAtSuperPosition,
  moveBeforeOrAfter,
  moveBlockBeforeOrAfter,
  pickPromoteGap,
  pickRowDrop,
  planDemoteOrder,
  sameCategoryDrop,
  sameSuperDrop,
  sameOrder,
  type GapBand,
  type RowBand,
} from "./dragOrder";

const row = (id: string, superKey: string, top: number, bottom: number): RowBand => ({
  id,
  superKey,
  top,
  bottom,
});

describe("pickRowDrop", () => {
  const rows = [row("a", "work", 100, 160), row("b", "work", 168, 228), row("c", "work", 236, 296)];

  it("returns null for an empty list", () => {
    expect(pickRowDrop([], 120)).toBeNull();
  });

  it("inserts before/after by the row midpoint", () => {
    expect(pickRowDrop(rows, 110)).toEqual({ rowId: "a", after: false });
    expect(pickRowDrop(rows, 150)).toEqual({ rowId: "a", after: true });
  });

  it("uses the nearest row when the cursor is in a gap", () => {
    expect(pickRowDrop(rows, 162)).toEqual({ rowId: "a", after: true });
    expect(pickRowDrop(rows, 166)).toEqual({ rowId: "b", after: false });
  });

  it("clamps above the first and below the last row", () => {
    expect(pickRowDrop(rows, 0)).toEqual({ rowId: "a", after: false });
    expect(pickRowDrop(rows, 999)).toEqual({ rowId: "c", after: true });
  });
});

describe("pickPromoteGap", () => {
  const gaps: GapBand[] = [
    { afterSuperId: "work", top: 300, bottom: 320, lineTop: 310 },
    { afterSuperId: "play", top: 500, bottom: 520, lineTop: 510 },
  ];

  it("uses the middle eight-pixel band", () => {
    expect(pickPromoteGap(gaps, 302)).toBe("work");
    expect(pickPromoteGap(gaps, 318)).toBe("work");
    expect(pickPromoteGap(gaps, 301)).toBeNull();
    expect(pickPromoteGap(gaps, 319)).toBeNull();
  });

  it("returns the matching gap after its super", () => {
    expect(pickPromoteGap(gaps, 510)).toBe("play");
  });
});

describe("drop intent equality", () => {
  it("compares category and super targets by identity", () => {
    expect(
      sameCategoryDrop(
        { kind: "row", superKey: "work", rowId: "a", after: true },
        { kind: "row", superKey: "work", rowId: "a", after: true },
      ),
    ).toBe(true);
    expect(
      sameCategoryDrop(
        { kind: "row", superKey: "work", rowId: "a", after: true },
        { kind: "row", superKey: "work", rowId: "a", after: false },
      ),
    ).toBe(false);
    expect(
      sameSuperDrop(
        { kind: "demote", targetSuperId: "play", targetRowId: null, after: false },
        { kind: "demote", targetSuperId: "play", targetRowId: null, after: false },
      ),
    ).toBe(true);
  });
});

describe("moveBeforeOrAfter", () => {
  const ids = ["a", "b", "c", "d"];

  it("moves down before and after the target", () => {
    expect(moveBeforeOrAfter(ids, "a", "c", false)).toEqual(["b", "a", "c", "d"]);
    expect(moveBeforeOrAfter(ids, "a", "c", true)).toEqual(["b", "c", "a", "d"]);
  });

  it("moves up before and after the target", () => {
    expect(moveBeforeOrAfter(ids, "d", "b", false)).toEqual(["a", "d", "b", "c"]);
    expect(moveBeforeOrAfter(ids, "d", "b", true)).toEqual(["a", "b", "d", "c"]);
  });

  it("is a no-op for the same id or an unknown target", () => {
    expect(moveBeforeOrAfter(ids, "b", "b", true)).toEqual(ids);
    expect(moveBeforeOrAfter(ids, "b", "missing", true)).toEqual(ids);
  });
});

describe("moveBlockBeforeOrAfter", () => {
  it("keeps the block's internal order", () => {
    const ids = ["a", "x", "b", "y", "c"];
    expect(moveBlockBeforeOrAfter(ids, ["x", "y"], "c", false)).toEqual(["a", "b", "x", "y", "c"]);
    expect(moveBlockBeforeOrAfter(ids, ["x", "y"], "a", true)).toEqual(["a", "x", "y", "b", "c"]);
  });
});

describe("insertBlockAtSuperPosition", () => {
  const superOf: Record<string, string> = {
    a: "work",
    b: "work",
    x: "work",
    p1: "play",
    p2: "play",
    z: "__builtin__",
  };
  const keyOf = (id: string) => superOf[id] ?? "__builtin__";
  const superOrder = ["work", "play", "__ORPHAN__"];

  it("inserts an empty target before the next super's rows", () => {
    expect(
      insertBlockAtSuperPosition(["a", "b", "x", "z"], ["x"], "play", superOrder, keyOf),
    ).toEqual(["a", "b", "x", "z"]);
  });

  it("appends after the target group's existing rows", () => {
    expect(
      insertBlockAtSuperPosition(["a", "b", "x", "p1", "p2"], ["x"], "play", superOrder, keyOf),
    ).toEqual(["a", "b", "p1", "p2", "x"]);
  });

  it("keeps builtins last when the target is the final group", () => {
    expect(
      insertBlockAtSuperPosition(
        ["a", "b", "x", "p1", "z"],
        ["x"],
        "__ORPHAN__",
        superOrder,
        keyOf,
      ),
    ).toEqual(["a", "b", "p1", "x", "z"]);
  });
});

describe("planDemoteOrder", () => {
  const keyOf = (id: string) =>
    id.startsWith("w") ? "work" : id.startsWith("p") ? "play" : "__ORPHAN__";
  const superOrder = ["work", "play", "__ORPHAN__"];

  it("inserts a block before or after a concrete row", () => {
    const ids = ["w1", "x", "y", "w2", "p1"];
    expect(planDemoteOrder(ids, ["x", "y"], "work", "w2", false, superOrder, keyOf)).toEqual([
      "w1",
      "x",
      "y",
      "w2",
      "p1",
    ]);
    expect(planDemoteOrder(ids, ["x", "y"], "work", "w2", true, superOrder, keyOf)).toEqual([
      "w1",
      "w2",
      "x",
      "y",
      "p1",
    ]);
  });

  it("inserts at the target super position when the body is empty", () => {
    const ids = ["w1", "x", "y", "p1"];
    expect(planDemoteOrder(ids, ["x", "y"], "play", null, false, superOrder, keyOf)).toEqual([
      "w1",
      "p1",
      "x",
      "y",
    ]);
  });
});

describe("insertSuperAfter", () => {
  it("inserts the promoted super immediately after its gap anchor", () => {
    expect(insertSuperAfter(["work", "play"], "work", "new")).toEqual(["work", "new", "play"]);
    expect(insertSuperAfter(["work", "play"], "missing", "new")).toBeNull();
  });
});

describe("sameOrder", () => {
  it("compares order and length", () => {
    expect(sameOrder(["a", "b"], ["a", "b"])).toBe(true);
    expect(sameOrder(["a", "b"], ["b", "a"])).toBe(false);
    expect(sameOrder(["a"], ["a", "b"])).toBe(false);
  });
});
