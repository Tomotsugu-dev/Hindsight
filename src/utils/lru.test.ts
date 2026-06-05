import { describe, expect, it } from "vitest";
import { lruInsert } from "./lru";

describe("lruInsert", () => {
  it("inserts entries under cap without eviction", () => {
    const m = new Map<string, number>();
    lruInsert(m, "a", 1, 3);
    lruInsert(m, "b", 2, 3);
    expect(m.size).toBe(2);
    expect(m.get("a")).toBe(1);
    expect(m.get("b")).toBe(2);
  });

  it("evicts oldest insertion when overflowing", () => {
    const m = new Map<string, number>();
    lruInsert(m, "a", 1, 3);
    lruInsert(m, "b", 2, 3);
    lruInsert(m, "c", 3, 3);
    lruInsert(m, "d", 4, 3);
    expect(m.size).toBe(3);
    expect(m.has("a")).toBe(false);
    expect([...m.keys()]).toEqual(["b", "c", "d"]);
  });

  it("re-set of existing key preserves insertion order", () => {
    const m = new Map<string, number>();
    lruInsert(m, "a", 1, 3);
    lruInsert(m, "b", 2, 3);
    lruInsert(m, "a", 99, 3); // 同 key 重写
    expect([...m.keys()]).toEqual(["a", "b"]);
    expect(m.get("a")).toBe(99);
  });

  it("re-set then overflow evicts original 'a' (oldest by insertion)", () => {
    const m = new Map<string, number>();
    lruInsert(m, "a", 1, 3);
    lruInsert(m, "b", 2, 3);
    lruInsert(m, "a", 99, 3); // 同 key 重写不改插入序
    lruInsert(m, "c", 3, 3);
    lruInsert(m, "d", 4, 3); // 触发 evict
    expect(m.has("a")).toBe(false);
    expect([...m.keys()]).toEqual(["b", "c", "d"]);
  });

  it("max=0 evicts immediately", () => {
    const m = new Map<string, number>();
    lruInsert(m, "a", 1, 0);
    expect(m.size).toBe(0);
  });
});
