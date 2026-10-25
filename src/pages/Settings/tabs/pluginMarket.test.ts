import { describe, it, expect } from "vitest";
import type { OnlinePlugin } from "../../../api/plugins";
import { filterMarket, matchesQuery, repoUrl } from "./pluginMarket";

const plugin: OnlinePlugin = {
  id: "hindsight-reminder",
  name: "网站与程序提醒",
  author: "Yoshino2000",
  description: {
    "zh-CN": "进入指定网站或程序时发送自定义通知",
    en: "Custom notifications when entering a website or app",
  },
  repo: "Yoshino2000/hindsight-reminder",
  versions: [],
  icon_url: null,
  official: false,
  download_count: 6,
};

describe("matchesQuery", () => {
  it("搜索框空着或只有空格，所有插件都显示", () => {
    expect(matchesQuery(plugin, "", "en")).toBe(true);
    expect(matchesQuery(plugin, "   ", "en")).toBe(true);
  });

  it("名称、作者、仓库里含有就算，不分大小写，前后空格不算", () => {
    expect(matchesQuery(plugin, "提醒", "en")).toBe(true);
    expect(matchesQuery(plugin, "yoshino", "en")).toBe(true);
    expect(matchesQuery(plugin, "  HINDSIGHT-REMINDER ", "en")).toBe(true);
  });

  it("简介只看界面语言那一种", () => {
    expect(matchesQuery(plugin, "通知", "zh-CN")).toBe(true);
    expect(matchesQuery(plugin, "通知", "en")).toBe(false);
    expect(matchesQuery(plugin, "notifications", "en")).toBe(true);
  });

  it("哪里都没有就不显示", () => {
    expect(matchesQuery(plugin, "pomodoro", "en")).toBe(false);
  });
});

describe("filterMarket", () => {
  // 后端已经按下载次数从多到少排好：focus 10、reminder 6、timer 3
  const focus = { ...plugin, id: "focus", name: "Focus", official: true };
  const reminder = plugin;
  const timer = { ...plugin, id: "timer", name: "Timer", official: true };
  const plugins = [focus, reminder, timer];
  const ids = (list: OnlinePlugin[]) => list.map((p) => p.id);

  it("下载最多：全部显示，顺序不变", () => {
    expect(ids(filterMarket(plugins, "popular", "", "en"))).toEqual([
      "focus",
      "hindsight-reminder",
      "timer",
    ]);
  });

  it("官方插件：只留官方的，顺序不变", () => {
    expect(ids(filterMarket(plugins, "official", "", "en"))).toEqual(["focus", "timer"]);
  });

  it("和搜索一起用时，两个条件都要满足", () => {
    expect(ids(filterMarket(plugins, "official", "timer", "en"))).toEqual(["timer"]);
    expect(ids(filterMarket(plugins, "official", "提醒", "en"))).toEqual([]);
  });
});

describe("repoUrl", () => {
  it("owner/name 拼成 GitHub 地址", () => {
    expect(repoUrl("Yoshino2000/hindsight-reminder")).toBe(
      "https://github.com/Yoshino2000/hindsight-reminder",
    );
    expect(repoUrl("someone/my.plugin_v2")).toBe("https://github.com/someone/my.plugin_v2");
  });

  it("格式不对的不给地址", () => {
    for (const repo of [
      "",
      "someone",
      "someone/",
      "/plugin",
      "a/b/c",
      "someone/..",
      "some one/plugin",
      "https://evil.example/x",
    ]) {
      expect(repoUrl(repo), repo).toBeNull();
    }
  });
});
