import { describe, expect, it } from "vitest";
import type { SiteRow } from "../../api/hindsight";
import { filterGroups, groupSites, ruleSource } from "./siteRows";

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

const shape = (groups: ReturnType<typeof groupSites>) =>
  groups.map((g) => [g.root.host, g.children.map((c) => c.host)]);

describe("groupSites", () => {
  it("subdomains go under the parent domain in the list, deeper ones too", () => {
    const groups = groupSites([
      row("live.bilibili.com"),
      row("bilibili.com"),
      row("api.live.bilibili.com"),
      row("github.com"),
    ]);
    expect(shape(groups)).toEqual([
      ["bilibili.com", ["live.bilibili.com", "api.live.bilibili.com"]],
      ["github.com", []],
    ]);
  });

  it("a subdomain whose parent domain is not in the list stays on its own", () => {
    expect(shape(groupSites([row("mail.google.com"), row("gemini.google.com")]))).toEqual([
      ["gemini.google.com", []],
      ["mail.google.com", []],
    ]);
  });

  it("orders groups by the time of the whole group", () => {
    // google.com alone has 30 min, but with gemini the group has 330 min.
    const groups = groupSites([
      row("youtube.com", { minutes30d: 120 }),
      row("google.com", { minutes30d: 30 }),
      row("gemini.google.com", { minutes30d: 300 }),
    ]);
    expect(groups.map((g) => g.root.host)).toEqual(["google.com", "youtube.com"]);
  });

  it("counts the children whose category differs from the parent's", () => {
    const [group] = groupSites([
      row("google.com", { categoryId: "browse" }),
      row("mail.google.com", { categoryId: "browse", follows: "google.com" }),
      row("gemini.google.com", { categoryId: "ai" }),
    ]);
    expect(group.otherCategoryCount).toBe(1);
  });

  it("does not group IP addresses", () => {
    expect(shape(groupSites([row("1.10"), row("192.168.1.10")]))).toEqual([
      ["1.10", []],
      ["192.168.1.10", []],
    ]);
  });
});

describe("filterGroups", () => {
  const groups = groupSites([
    row("github.com"),
    row("gist.github.com"),
    row("bilibili.com"),
    row("live.bilibili.com"),
  ]);
  const shown = (search: string) =>
    filterGroups(groups, search).map((v) => [
      v.group.root.host,
      v.children.map((c) => c.host),
      v.forceOpen,
    ]);

  it("an empty or blank search shows every group as it is", () => {
    expect(shown("   ")).toEqual([
      ["bilibili.com", ["live.bilibili.com"], false],
      ["github.com", ["gist.github.com"], false],
    ]);
  });

  it("a matching website keeps all its children, ignoring case", () => {
    expect(shown(" GitHub ")).toEqual([["github.com", ["gist.github.com"], false]]);
  });

  it("a match on a child alone opens its group and shows only that child", () => {
    expect(shown("live")).toEqual([["bilibili.com", ["live.bilibili.com"], true]]);
  });

  it("shows nothing when no website matches", () => {
    expect(shown("youtube")).toEqual([]);
  });
});
