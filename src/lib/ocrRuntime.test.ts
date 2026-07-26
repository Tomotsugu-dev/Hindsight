import { beforeEach, describe, expect, it, vi } from "vitest";

// 不用 vi.fn 做拒绝路径的桩:vitest 会给 mock 结果挂 settledResults 追踪,
// 对 rejected promise 会派生出一条无人消费的拒绝链,把本已被 catch 的用例
// 误判成 unhandled rejection。改用可替换的普通函数,行为零插桩。
const h = vi.hoisted(() => ({
  impl: (): Promise<unknown> => Promise.resolve({}),
}));
vi.mock("../api/hindsight", () => ({
  api: { getEngineStatus: () => h.impl() },
}));
import { ocrRuntimeReady } from "./ocrRuntime";

describe("ocrRuntimeReady", () => {
  beforeEach(() => {
    h.impl = () => Promise.resolve({});
  });

  it("macOS 平台恒就绪(系统 Vision)", async () => {
    h.impl = () =>
      Promise.resolve({
        platformId: "macos-arm64",
        embeddingRuntime: { installed: false },
      });
    expect(await ocrRuntimeReady()).toBe(true);
  });

  it("Windows 看运行时安装状态", async () => {
    h.impl = () =>
      Promise.resolve({
        platformId: "windows-x64",
        embeddingRuntime: { installed: false },
      });
    expect(await ocrRuntimeReady()).toBe(false);
    h.impl = () =>
      Promise.resolve({
        platformId: "windows-x64",
        embeddingRuntime: { installed: true },
      });
    expect(await ocrRuntimeReady()).toBe(true);
  });

  it("查询失败按就绪放行,让后续动作走原错误链路", async () => {
    h.impl = () => Promise.reject(new Error("ipc down"));
    expect(await ocrRuntimeReady()).toBe(true);
  });
});
