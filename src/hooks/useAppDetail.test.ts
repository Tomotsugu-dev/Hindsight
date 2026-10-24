import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { AppTitles, DetailBucket } from "../api/hindsight";

// 缓存在模块顶层，每个测试重新加载模块，拿到一份空缓存和新的接口桩
vi.mock("../api/hindsight", () => ({
  api: {
    getAppBuckets: vi.fn(),
    getAppTitles: vi.fn(),
  },
}));

async function load() {
  vi.resetModules();
  const { api } = await import("../api/hindsight");
  const mod = await import("./useAppDetail");
  return { api: vi.mocked(api), ...mod };
}

function buckets(secs: number): DetailBucket[] {
  return [{ key: "2026-10-21", secs }];
}

function titles(secs: number): AppTitles {
  return { titles: [{ title: "main.rs", secs, host: null }], isBrowser: false };
}

describe("每天的时长和标题的缓存", () => {
  beforeEach(() => {
    // 接口桩的调用次数和返回值跨测试会累积
    vi.resetAllMocks();
    vi.useFakeTimers();
    vi.setSystemTime(new Date("2026-10-24T10:00:00"));
  });
  afterEach(() => {
    vi.useRealTimers();
  });

  it("悬停后点开：每天的时长用悬停查过的，只请求标题", async () => {
    const { api, loadAppBuckets, loadAppDetail } = await load();
    api.getAppBuckets.mockResolvedValue(buckets(600));
    api.getAppTitles.mockResolvedValue(titles(600));

    await loadAppBuckets("month", 0, "chrome", undefined);
    const detail = await loadAppDetail("month", 0, "chrome", undefined);

    expect(detail).toEqual({ buckets: buckets(600), ...titles(600) });
    expect(api.getAppBuckets).toHaveBeenCalledTimes(1);
    expect(api.getAppTitles).toHaveBeenCalledTimes(1);
  });

  it("悬停的请求还没回来时点开：等同一个请求，不再发一次", async () => {
    const { api, loadAppBuckets, loadAppDetail } = await load();
    let resolve!: (b: DetailBucket[]) => void;
    api.getAppBuckets.mockReturnValue(new Promise((r) => (resolve = r)));
    api.getAppTitles.mockResolvedValue(titles(600));

    const hover = loadAppBuckets("month", 0, "chrome", undefined);
    const drawer = loadAppDetail("month", 0, "chrome", undefined);
    resolve(buckets(600));

    expect(await hover).toEqual(buckets(600));
    expect((await drawer).buckets).toEqual(buckets(600));
    expect(api.getAppBuckets).toHaveBeenCalledTimes(1);
  });

  it("查过的直接从缓存拿，不再请求", async () => {
    const { api, loadAppBuckets, cachedAppBuckets } = await load();
    api.getAppBuckets.mockResolvedValue(buckets(600));

    expect(cachedAppBuckets("month", -1, "chrome", undefined)).toBeNull();
    await loadAppBuckets("month", -1, "chrome", undefined);

    expect(cachedAppBuckets("month", -1, "chrome", undefined)).toEqual(buckets(600));
    expect(await loadAppBuckets("month", -1, "chrome", undefined)).toEqual(buckets(600));
    expect(api.getAppBuckets).toHaveBeenCalledTimes(1);
  });

  it("本期的缓存 30 秒后过期，重新请求；以前的期不过期", async () => {
    const { api, loadAppBuckets, cachedAppBuckets } = await load();
    api.getAppBuckets
      .mockResolvedValueOnce(buckets(600))
      .mockResolvedValueOnce(buckets(300))
      .mockResolvedValueOnce(buckets(900));
    await loadAppBuckets("month", 0, "chrome", undefined);
    await loadAppBuckets("month", -1, "chrome", undefined);

    vi.advanceTimersByTime(31_000);

    expect(cachedAppBuckets("month", 0, "chrome", undefined)).toBeNull();
    expect(await loadAppBuckets("month", 0, "chrome", undefined)).toEqual(buckets(900));
    expect(cachedAppBuckets("month", -1, "chrome", undefined)).toEqual(buckets(300));
    expect(api.getAppBuckets).toHaveBeenCalledTimes(3);
  });

  it("过了午夜清空缓存：offset 指向的日期变了", async () => {
    const { api, loadAppBuckets, cachedAppBuckets } = await load();
    api.getAppBuckets.mockResolvedValue(buckets(600));
    await loadAppBuckets("month", -1, "chrome", undefined);

    vi.setSystemTime(new Date("2026-10-25T00:00:01"));

    expect(cachedAppBuckets("month", -1, "chrome", undefined)).toBeNull();
  });

  it("请求失败不缓存，下次重新请求", async () => {
    const { api, loadAppBuckets } = await load();
    api.getAppBuckets.mockRejectedValueOnce(new Error("db")).mockResolvedValueOnce(buckets(600));

    await expect(loadAppBuckets("month", 0, "chrome", undefined)).rejects.toThrow("db");
    expect(await loadAppBuckets("month", 0, "chrome", undefined)).toEqual(buckets(600));
    expect(api.getAppBuckets).toHaveBeenCalledTimes(2);
  });

  it("抽屉里标题查失败：柱子照常显示，标题当作空的，下次重新请求标题", async () => {
    const { api, loadAppDetail } = await load();
    api.getAppBuckets.mockResolvedValue(buckets(600));
    api.getAppTitles.mockRejectedValueOnce(new Error("db")).mockResolvedValueOnce(titles(600));

    expect(await loadAppDetail("month", 0, "chrome", undefined)).toEqual({
      buckets: buckets(600),
      titles: [],
      isBrowser: false,
    });
    expect(await loadAppDetail("month", 0, "chrome", undefined)).toEqual({
      buckets: buckets(600),
      ...titles(600),
    });
    expect(api.getAppTitles).toHaveBeenCalledTimes(2);
  });

  it("应用、范围、期、设备不同的，各查各的", async () => {
    const { api, loadAppBuckets } = await load();
    api.getAppBuckets.mockResolvedValue(buckets(600));

    await loadAppBuckets("month", 0, "chrome", undefined);
    await loadAppBuckets("month", 0, "code", undefined);
    await loadAppBuckets("month", -1, "chrome", undefined);
    await loadAppBuckets("month", 0, "chrome", "device-win");
    await loadAppBuckets("week", 0, "chrome", undefined);

    expect(api.getAppBuckets).toHaveBeenCalledTimes(5);
  });
});
