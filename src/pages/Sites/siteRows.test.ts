import { describe, expect, it } from "vitest";
import type { SiteRow } from "../../api/hindsight";
import { filterSites, ruleSource } from "./siteRows";

const row = (host: string, patch: Partial<SiteRow> = {}): SiteRow => ({
  host,
  minutes30d: 0,
  minutesTotal: 0,
  categoryId: null,
  follows: null,
  ...patch,
});

describe("ruleSource", () => {
  it("a category without a parent domain comes from the website's own rule", () => {
    expect(ruleSource(row("bilibili.com", { categoryId: "fun" }))).toBe("own");
  });

  it("a category with a parent domain is inherited", () => {
    expect(
      ruleSource(row("live.bilibili.com", { categoryId: "fun", follows: "bilibili.com" })),
    ).toBe("parent");
  });

  it("no category means no rule applies", () => {
    expect(ruleSource(row("github.com"))).toBe("none");
  });
});

describe("filterSites", () => {
  const rows = [row("github.com"), row("gist.github.com"), row("bilibili.com")];

  it("an empty or blank search returns the same array", () => {
    expect(filterSites(rows, "")).toBe(rows);
    expect(filterSites(rows, "   ")).toBe(rows);
  });

  it("matches part of the host, ignoring case and surrounding spaces, in input order", () => {
    expect(filterSites(rows, " GitHub ").map((r) => r.host)).toEqual([
      "github.com",
      "gist.github.com",
    ]);
  });

  it("returns nothing when no host matches", () => {
    expect(filterSites(rows, "youtube")).toEqual([]);
  });
});
