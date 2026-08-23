import { describe, it, expect } from "vitest";
import { groupBySite } from "./groupBySite";

describe("groupBySite", () => {
  it("按域名分组、组内同标题合并、组和页面都按秒数降序", () => {
    const groups = groupBySite([
      { title: "Hindsight", secs: 600, host: "github.com" },
      { title: "Issues", secs: 300, host: "github.com" },
      { title: "Hindsight", secs: 300, host: "github.com" }, // 同域同标题 → 合并成 900
      { title: "视频 A", secs: 1800, host: "youtube.com" },
    ]);
    expect(groups.map((g) => g.host)).toEqual(["youtube.com", "github.com"]);
    expect(groups[1].secs).toBe(1200);
    expect(groups[1].pages).toEqual([
      { title: "Hindsight", secs: 900 },
      { title: "Issues", secs: 300 },
    ]);
  });

  it("同标题不同域名分到各自的组，不串", () => {
    const groups = groupBySite([
      { title: "首页", secs: 30, host: "github.com" },
      { title: "首页", secs: 45, host: "youtube.com" },
    ]);
    expect(groups).toHaveLength(2);
    expect(groups[0]).toMatchObject({ host: "youtube.com", secs: 45 });
    expect(groups[1]).toMatchObject({ host: "github.com", secs: 30 });
  });

  it("无域名的行归入 host=null 组且固定排最后，哪怕它时长最大", () => {
    const groups = groupBySite([
      { title: "旧记录", secs: 9999, host: null },
      { title: "空串也算无域名", secs: 1, host: "" },
      { title: "Hindsight", secs: 10, host: "github.com" },
    ]);
    expect(groups.map((g) => g.host)).toEqual(["github.com", null]);
    expect(groups[1].secs).toBe(10000);
    expect(groups[1].pages).toHaveLength(2);
  });

  it("空输入返回空数组", () => {
    expect(groupBySite([])).toEqual([]);
  });
});
