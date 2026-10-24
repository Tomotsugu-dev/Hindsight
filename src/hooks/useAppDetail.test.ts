import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { AppDetail } from "../api/hindsight";

// 缓存在模块顶层，每个测试重新加载模块，拿到一份空缓存和新的接口桩
vi.mock("../api/hindsight", () => ({
  api: {
    getAppDayDetail: vi.fn(),
    getAppWeekDetail: vi.fn(),
    getAppMonthDetail: vi.fn(),
  },
}));

async function load() {
  vi.resetModules();
  const { api } = await import("../api/hindsight");
  const mod = await import("./useAppDetail");
  return { api: vi.mocked(api), ...mod };
}

function detail(secs: number): AppDetail {
  return { buckets: [{ key: "2026-10-21", secs }], titles: [], isBrowser: false };
}

describe("loadAppDetail / cachedAppDetail", () => {
  beforeEach(() => {
    // 接口桩的调用次数和返回值跨测试会累积
    vi.resetAllMocks();
    vi.useFakeTimers();
    vi.setSystemTime(new Date("2026-10-24T10:00:00"));
  });
  afterEach(() => {
    vi.useRealTimers();
  });

  it("悬停的请求还没回来时点开抽屉：等同一个请求，不再发一次", async () => {
    const { api, loadAppDetail } = await load();
    let resolve!: (d: AppDetail) => void;
    api.getAppWeekDetail.mockReturnValue(new Promise((r) => (resolve = r)));

    const hover = loadAppDetail("week", 0, "chrome", undefined);
    const drawer = loadAppDetail("week", 0, "chrome", undefined);
    resolve(detail(600));

    expect(await hover).toEqual(detail(600));
    expect(await drawer).toEqual(detail(600));
    expect(api.getAppWeekDetail).toHaveBeenCalledTimes(1);
  });

  it("查过的直接从缓存拿，不再请求", async () => {
    const { api, loadAppDetail, cachedAppDetail } = await load();
    api.getAppMonthDetail.mockResolvedValue(detail(600));

    expect(cachedAppDetail("month", -1, "chrome", undefined)).toBeNull();
    await loadAppDetail("month", -1, "chrome", undefined);

    expect(cachedAppDetail("month", -1, "chrome", undefined)).toEqual(detail(600));
    expect(await loadAppDetail("month", -1, "chrome", undefined)).toEqual(detail(600));
    expect(api.getAppMonthDetail).toHaveBeenCalledTimes(1);
  });

  it("当前期的缓存 30 秒后过期，重新请求；以前的期不过期", async () => {
    const { api, loadAppDetail, cachedAppDetail } = await load();
    api.getAppDayDetail.mockResolvedValueOnce(detail(600)).mockResolvedValueOnce(detail(900));
    api.getAppWeekDetail.mockResolvedValue(detail(300));
    await loadAppDetail("day", 0, "chrome", undefined);
    await loadAppDetail("week", -1, "chrome", undefined);

    vi.advanceTimersByTime(31_000);

    expect(cachedAppDetail("day", 0, "chrome", undefined)).toBeNull();
    expect(await loadAppDetail("day", 0, "chrome", undefined)).toEqual(detail(900));
    expect(cachedAppDetail("week", -1, "chrome", undefined)).toEqual(detail(300));
    expect(api.getAppDayDetail).toHaveBeenCalledTimes(2);
    expect(api.getAppWeekDetail).toHaveBeenCalledTimes(1);
  });

  it("过了午夜清空缓存：offset 指向的日期变了", async () => {
    const { api, loadAppDetail, cachedAppDetail } = await load();
    api.getAppWeekDetail.mockResolvedValue(detail(600));
    await loadAppDetail("week", -1, "chrome", undefined);

    vi.setSystemTime(new Date("2026-10-25T00:00:01"));

    expect(cachedAppDetail("week", -1, "chrome", undefined)).toBeNull();
  });

  it("请求失败不缓存，下次重新请求", async () => {
    const { api, loadAppDetail } = await load();
    api.getAppWeekDetail.mockRejectedValueOnce(new Error("db")).mockResolvedValueOnce(detail(600));

    await expect(loadAppDetail("week", 0, "chrome", undefined)).rejects.toThrow("db");
    expect(await loadAppDetail("week", 0, "chrome", undefined)).toEqual(detail(600));
    expect(api.getAppWeekDetail).toHaveBeenCalledTimes(2);
  });

  it("应用、范围、期、设备不同的，各查各的", async () => {
    const { api, loadAppDetail } = await load();
    api.getAppWeekDetail.mockResolvedValue(detail(600));
    api.getAppMonthDetail.mockResolvedValue(detail(600));

    await loadAppDetail("week", 0, "chrome", undefined);
    await loadAppDetail("week", 0, "code", undefined);
    await loadAppDetail("week", -1, "chrome", undefined);
    await loadAppDetail("week", 0, "chrome", "device-win");
    await loadAppDetail("month", 0, "chrome", undefined);

    expect(api.getAppWeekDetail).toHaveBeenCalledTimes(4);
    expect(api.getAppMonthDetail).toHaveBeenCalledTimes(1);
  });
});
