import { afterEach, describe, expect, it, vi } from "vitest";
import { logError, logWarn } from "./logger";

afterEach(() => vi.restoreAllMocks());

describe("logger", () => {
  it("logError 以 [scope] 前缀落 console.error", () => {
    const spy = vi.spyOn(console, "error").mockImplementation(() => {});
    const err = new Error("boom");
    logError("settings.load", err);
    expect(spy).toHaveBeenCalledWith("[settings.load]", err);
  });
  it("logWarn 以 [scope] 前缀落 console.warn", () => {
    const spy = vi.spyOn(console, "warn").mockImplementation(() => {});
    logWarn("chat.poll", "slow");
    expect(spy).toHaveBeenCalledWith("[chat.poll]", "slow");
  });
});
