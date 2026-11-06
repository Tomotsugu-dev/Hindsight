// 缓存是模块里的状态，每个用例 resetModules 后重新 import 一份干净的。
// 时间用假时钟控制；请求用手动 resolve / reject 的 Promise，看它什么时候发出去。
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

async function freshImport() {
  vi.resetModules();
  return import("./insightQueries");
}

/** 一个请求：记下被调用了几次，结果由测试手动给 */
function pending(key: string, live = false) {
  const calls: { resolve: (v: unknown) => void; reject: (e: unknown) => void }[] = [];
  return {
    key,
    live,
    run: () => new Promise<unknown>((resolve, reject) => calls.push({ resolve, reject })),
    calls,
  };
}

const flush = () => new Promise((r) => setTimeout(r, 0));

beforeEach(() => {
  vi.useFakeTimers({ toFake: ["Date"] });
  vi.setSystemTime(new Date(2026, 10, 5, 15, 40));
  vi.spyOn(console, "warn").mockImplementation(() => {});
});

afterEach(() => {
  vi.useRealTimers();
  vi.restoreAllMocks();
});

describe("insightQueries", () => {
  it("runs at most two at once, in the order asked", async () => {
    const m = await freshImport();
    const [a, b, c] = [pending("a"), pending("b"), pending("c")];
    m.requestQueries([a, b, c]);
    expect([a.calls.length, b.calls.length, c.calls.length]).toEqual([1, 1, 0]);
    a.calls[0].resolve(1);
    await flush();
    expect(c.calls).toHaveLength(1);
    expect(m.peekQuery("a")).toBe(1);
  });

  it("drops the queries still waiting when new ones are asked for", async () => {
    const m = await freshImport();
    const [a, b, old, next] = [pending("a"), pending("b"), pending("old"), pending("next")];
    m.requestQueries([a, b, old]);
    // 翻到别的周：old 还没发出去，就不再发
    m.requestQueries([next]);
    a.calls[0].resolve(1);
    b.calls[0].resolve(2);
    await flush();
    expect(old.calls).toHaveLength(0);
    expect(next.calls).toHaveLength(1);
  });

  it("asks a live query again after 30 seconds and shows the old result meanwhile", async () => {
    const m = await freshImport();
    const today = pending("today", true);
    const past = pending("past");
    m.requestQueries([today, past]);
    today.calls[0].resolve("v1");
    past.calls[0].resolve("p");
    await flush();

    vi.setSystemTime(new Date(2026, 10, 5, 15, 40, 31));
    m.requestQueries([today, past]);
    expect(today.calls).toHaveLength(2);
    expect(past.calls).toHaveLength(1); // 过去的数据不再问
    expect(m.peekQuery("today")).toBe("v1");
    today.calls[1].resolve("v2");
    await flush();
    expect(m.peekQuery("today")).toBe("v2");
  });

  it("waits 30 seconds before asking a failed query again", async () => {
    const m = await freshImport();
    const q = pending("q");
    m.requestQueries([q]);
    q.calls[0].reject(new Error("db busy"));
    await flush();
    expect(console.warn).toHaveBeenCalledTimes(1);

    // 每次渲染都会再要一遍，出错后不能每次都发
    m.requestQueries([q]);
    expect(q.calls).toHaveLength(1);
    vi.setSystemTime(new Date(2026, 10, 5, 15, 40, 30));
    m.requestQueries([q]);
    expect(q.calls).toHaveLength(2);
  });

  it("drops results sent before the categories changed", async () => {
    const m = await freshImport();
    const q = pending("q");
    m.clearOnNewCategories(["first list"]);
    m.requestQueries([q]);
    m.clearOnNewCategories(["second list"]);
    q.calls[0].resolve("old totals");
    await flush();
    expect(m.peekQuery("q")).toBeUndefined();
  });

  it("starts over on a new day", async () => {
    const m = await freshImport();
    const q = pending("q");
    m.requestQueries([q]);
    q.calls[0].resolve("yesterday");
    await flush();
    vi.setSystemTime(new Date(2026, 10, 6, 0, 1));
    m.requestQueries([q]);
    expect(m.peekQuery("q")).toBeUndefined();
    expect(q.calls).toHaveLength(2);
  });
});
