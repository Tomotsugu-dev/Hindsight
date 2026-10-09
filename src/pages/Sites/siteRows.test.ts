import { describe, expect, it } from "vitest";
import type { SiteRow } from "../../api/hindsight";
import { filterGroups, groupSites, ruleSource, sortGroups } from "./siteRows";

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

describe("website category filters", () => {
  const groups = groupSites([
    row("google.com", { categoryId: "browse" }),
    row("mail.google.com", { categoryId: "browse", follows: "google.com" }),
    row("gemini.google.com", { categoryId: "ai" }),
    row("youtube.com", { categoryId: "fun" }),
    row("github.com"),
  ]);

  it("accepts several categories, including categories inherited from a parent", () => {
    const visible = filterGroups(groups, "", { selectedCategoryIds: ["browse", "fun"] });
    expect(visible.map((v) => [v.group.root.host, v.children.map((c) => c.host)])).toEqual([
      ["google.com", ["mail.google.com"]],
      ["youtube.com", []],
    ]);
  });

  it("opens a parent for a matching child without including nonmatching siblings", () => {
    const [visible] = filterGroups(groups, "", { selectedCategoryIds: ["ai"] });
    expect(visible.group.root.host).toBe("google.com");
    expect(visible.children.map((c) => c.host)).toEqual(["gemini.google.com"]);
    expect(visible.forceOpen).toBe(true);
    expect(visible.rootMatches).toBe(false);
  });

  it("does not include children from other categories when their parent matches", () => {
    const [visible] = filterGroups(groups, "google", { selectedCategoryIds: ["browse"] });
    expect(visible.rootMatches).toBe(true);
    expect(visible.children.map((c) => c.host)).toEqual(["mail.google.com"]);
  });

  it("combines the website search with the selected categories", () => {
    expect(filterGroups(groups, "youtube", { selectedCategoryIds: ["browse"] })).toEqual([]);
    const visible = filterGroups(groups, " MAIL ", { selectedCategoryIds: ["browse"] });
    expect(visible[0].children.map((c) => c.host)).toEqual(["mail.google.com"]);
    expect(visible[0].forceOpen).toBe(true);
  });

  it("unassigned mode takes precedence over selected categories", () => {
    const visible = filterGroups(groups, "", {
      selectedCategoryIds: ["browse"],
      unassignedOnly: true,
    });
    expect(visible.map((v) => v.group.root.host)).toEqual(["github.com"]);
  });

  it("keeps an assigned parent only as context for an unassigned child", () => {
    const input = groupSites([row("example.com", { categoryId: "work" }), row("new.example.com")]);
    const [visible] = filterGroups(input, "", { unassignedOnly: true });
    expect(visible.rootMatches).toBe(false);
    expect(visible.forceOpen).toBe(true);
    expect(visible.children.map((c) => c.host)).toEqual(["new.example.com"]);
  });

  it("does not change the original groups while filtering", () => {
    const original = structuredClone(groups);
    filterGroups(groups, "", { selectedCategoryIds: ["ai"] });
    expect(groups).toEqual(original);
  });
});

describe("website sorting", () => {
  const groups = filterGroups(
    groupSites([
      row("example.com", { minutes30d: 10, minutesTotal: 100 }),
      row("b.example.com", { minutes30d: 70, minutesTotal: 200 }),
      row("a.example.com", { minutes30d: 20, minutesTotal: 400 }),
      row("youtube.com", { minutes30d: 50, minutesTotal: 900 }),
    ]),
    "",
  );
  const hosts = (sortBy: Parameters<typeof sortGroups>[1]) =>
    sortGroups(groups, sortBy).map((group) => group.group.root.host);

  it("sorts by the matching group's recent time, including its subdomains", () => {
    expect(hosts("recentDesc")).toEqual(["example.com", "youtube.com"]);
    expect(hosts("recentAsc")).toEqual(["youtube.com", "example.com"]);
  });

  it("can sort by all-time usage independently of recent usage", () => {
    expect(hosts("totalDesc")).toEqual(["youtube.com", "example.com"]);
    expect(hosts("totalAsc")).toEqual(["example.com", "youtube.com"]);
  });

  it("sorts both parent domains and subdomains alphabetically", () => {
    const ascending = sortGroups(groups, "nameAsc");
    expect(ascending.map((v) => v.group.root.host)).toEqual(["example.com", "youtube.com"]);
    expect(ascending[0].children.map((c) => c.host)).toEqual(["a.example.com", "b.example.com"]);
    const descending = sortGroups(groups, "nameDesc");
    expect(descending.map((v) => v.group.root.host)).toEqual(["youtube.com", "example.com"]);
    expect(descending[1].children.map((c) => c.host)).toEqual(["b.example.com", "a.example.com"]);
  });

  it("does not include a parent kept only as context in the filtered sorting total", () => {
    const filtered = filterGroups(
      groupSites([
        row("example.com", { categoryId: "browse", minutes30d: 5000 }),
        row("work.example.com", { categoryId: "work", minutes30d: 10 }),
        row("github.com", { categoryId: "work", minutes30d: 100 }),
      ]),
      "",
      { selectedCategoryIds: ["work"] },
    );
    expect(sortGroups(filtered, "recentDesc").map((v) => v.group.root.host)).toEqual([
      "github.com",
      "example.com",
    ]);
  });

  it("sorts child rows by time and keeps the default order and input untouched", () => {
    const original = structuredClone(groups);
    expect(sortGroups(groups, "recentAsc")[1].children.map((c) => c.host)).toEqual([
      "a.example.com",
      "b.example.com",
    ]);
    expect(sortGroups(groups, "default")).toEqual(original);
    expect(groups).toEqual(original);
  });
});
