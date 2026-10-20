import { describe, expect, it } from "vitest";
import { isRequest, originOf } from "./forwarder";

describe("originOf", () => {
  it("cuts the scheme and host of a plugin page URL", () => {
    expect(originOf("hindsight-plugin://focus.localhost/0.1.0/ui/index.html")).toBe(
      "hindsight-plugin://focus.localhost",
    );
    // The URL on Windows
    expect(originOf("http://hindsight-plugin.focus.localhost/0.1.0/ui/index.html")).toBe(
      "http://hindsight-plugin.focus.localhost",
    );
  });
});

describe("isRequest", () => {
  it("needs a numeric id and a method name", () => {
    expect(isRequest({ id: 1, method: "stats.day" })).toBe(true);
    expect(isRequest({ id: 1, method: "stats.day", params: { date: "x" } })).toBe(true);
    expect(isRequest({ id: "1", method: "stats.day" })).toBe(false);
    expect(isRequest({ id: 1 })).toBe(false);
    expect(isRequest(null)).toBe(false);
    expect(isRequest("call")).toBe(false);
  });
});
