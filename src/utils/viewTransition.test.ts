import { afterEach, describe, expect, it, vi } from "vitest";
import { withViewTransition } from "./viewTransition";

afterEach(() => vi.unstubAllGlobals());

describe("withViewTransition", () => {
  it("无 document(SSR)直接执行回调", () => {
    let ran = 0;
    withViewTransition(() => ran++);
    expect(ran).toBe(1);
  });
  it("有 document 但无 startViewTransition(老浏览器)退化为直接执行", () => {
    vi.stubGlobal("document", {});
    let ran = 0;
    withViewTransition(() => ran++);
    expect(ran).toBe(1);
  });
  it("有 startViewTransition:回调在过渡内被执行一次", () => {
    let ran = 0;
    const start = vi.fn((cb: () => void) => {
      cb();
      return {};
    });
    vi.stubGlobal("document", { startViewTransition: start });
    withViewTransition(() => ran++);
    expect(start).toHaveBeenCalledTimes(1);
    expect(ran).toBe(1);
  });
});
