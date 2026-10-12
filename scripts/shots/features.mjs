#!/usr/bin/env node
/**
 * 给 docs/features/zh 下的功能文档截图，每份文档的图存到 docs/features/zh/imgs/<文档名>/。
 *
 * npm run shots:features
 * npm run shots:features -- --only month-pie,categories --theme dark
 * npm run shots:features -- --only search-results,search-preview
 *
 * 构建 demo、打开页面这些步骤在 web-demo.mjs，和 readme/shoot.mjs 共用。要截哪几张见 SHOTS。
 */

import { mkdir } from "node:fs/promises";
import path from "node:path";
import { parseArgs } from "node:util";
import {
  CHAT_QUESTIONS,
  demoOptions,
  repo,
  scopes,
  selectShots,
  shoot,
  withDemo,
} from "./web-demo.mjs";

/**
 * 文档里要的图：属于哪份文档（doc，即 imgs 下的文件夹名）、文件名、哪一页。
 * 统计页还要指定时段（bars）还是占比（pie）；占比视图还要指定哪一层：
 * supers 只有大类的环，cats 往下滚出来的小类同心环。pick 是截图前先点的大类和小类。
 * toCards：不要侧栏，只截到统计卡片那一行。
 */
const SHOTS = [
  { doc: "activity-record", file: "day-bars", page: "day", view: "bars" },
  { doc: "activity-record", file: "week-bars", page: "week", view: "bars" },
  { doc: "activity-record", file: "month-bars", page: "month", view: "bars" },
  {
    doc: "activity-record",
    file: "month-pie",
    page: "month",
    view: "pie",
    depth: "cats",
    toCards: true,
  },
  {
    doc: "activity-record",
    file: "month-pie-selected",
    page: "month",
    view: "pie",
    depth: "cats",
    toCards: true,
    // 只钉住「工作」大类，不选下面的小类
    pick: { superId: "work" },
  },
  // 全部历史：图表卡里年表 / 月表两个标签各一张，全窗口
  { doc: "activity-record", file: "alltime-year", page: "history", historyTab: "year" },
  { doc: "activity-record", file: "alltime-month", page: "history", historyTab: "month" },
  // 「分类」页的「管理分类」：左边是大类，右边一行一个小类，行里是应用
  { doc: "classification-app-management", file: "categories", page: "categories" },
  // 「分类」页的「归类应用」
  { doc: "classification-app-management", file: "apps", page: "categories", tab: "apps" },
  // 「分类」页的「归类网站」
  { doc: "classification-app-management", file: "sites", page: "categories", tab: "sites" },
  // 「全部历史 → 搜索」：搜索 OCR，再打开维基百科的文字识别词条截图。
  {
    doc: "screen-memory-search",
    file: "search-results",
    page: "search",
    query: "OCR",
    resultTitle: "Optical character recognition — Wikipedia",
  },
  {
    doc: "screen-memory-search",
    file: "search-preview",
    page: "search",
    query: "OCR",
    resultTitle: "Optical character recognition — Wikipedia",
    preview: true,
    highlights: true,
  },
  // 「AI 总结 → 日报」：demo 自带当天分时段的报告
  { doc: "ai-summary-and-chat", file: "daily-report", page: "aiSummary", ready: "HourlyChart" },
  // 「对话」：问一个问题，回答和来源出来后截图。
  { doc: "ai-summary-and-chat", file: "chat", page: "chat", questions: CHAT_QUESTIONS },
  // 「AI 设置 → 常规」：只要时段划分和过滤，定时计划不要。滚到底，裁掉侧栏。
  {
    doc: "ai-configuration",
    file: "summary-settings",
    page: "aiSettings",
    tab: "general",
    scrollToEnd: true,
    sections: ["aiSettings.segments.sectionTitle", "aiSettings.filter.sectionTitle"],
  },
  // 日统计页面在三种主题下的样子，全窗口
  {
    doc: "interface-and-appearance",
    file: "day-default",
    page: "day",
    view: "bars",
    theme: "default",
  },
  {
    doc: "interface-and-appearance",
    file: "day-minimal",
    page: "day",
    view: "bars",
    theme: "minimal",
  },
  {
    doc: "interface-and-appearance",
    file: "day-dark",
    page: "day",
    view: "bars",
    theme: "dark",
  },
];

async function options() {
  const { values } = parseArgs({
    options: {
      only: { type: "string" },
      out: { type: "string", default: "docs/features/zh/imgs" },
      lang: { type: "string", default: "zh-CN" },
      theme: { type: "string", default: "minimal" },
      date: { type: "string" },
      help: { type: "boolean", short: "h" },
    },
  });
  if (values.help) {
    console.log(`用法：npm run shots:features -- [选项]
  --only NAME,NAME               只截这几张，默认全截：${SHOTS.map((s) => s.file).join("、")}
  --out PATH                     输出目录，默认 docs/features/zh/imgs，每份文档一个子文件夹
  --lang LOCALE                  界面语言，默认 zh-CN
  --theme minimal|default|dark   外观，默认 minimal
  --date YYYY-MM-DD              演示数据的「今天」，默认东京当天

窗口尺寸取 src-tauri/tauri.macos.conf.json。需要 Google Chrome，
或先执行 npx playwright install chromium。`);
    return null;
  }

  const shots = selectShots(SHOTS, values.only);
  return { ...(await demoOptions(values)), shots, out: path.resolve(repo, values.out) };
}

/**
 * 不要侧栏、只截到统计卡片那一行的范围：左边从内容区开始，下边停在统计卡片和下面
 * 「最常用应用」之间的空隙正中。统计卡片用「较上期」那张卡片的标签找。
 *
 * 侧栏的投影会落到内容区左边，所以先把侧栏藏起来；下一张图会重新载入页面。
 */
async function cardsClip(page, shot, labels) {
  const scope = scopes[shot.page];
  await page
    .locator("aside")
    .first()
    .evaluate((aside) => {
      aside.style.visibility = "hidden";
    });
  const main = await page.locator("main").boundingBox();
  const cards = await page
    .getByText(labels[scope].insights.vsPrev.label, { exact: true })
    .first()
    .locator("xpath=../..")
    .boundingBox();
  const appListTitle = [labels[scope].ranks.topApps, labels.today.pie.drill.appsTitle];
  let appList = null;
  for (const name of appListTitle) {
    const heading = page.getByRole("heading", { name, exact: true });
    if (await heading.count()) {
      appList = heading.locator("xpath=ancestor::section[1]");
      break;
    }
  }
  if (!appList) throw new Error("找不到应用列表，无法裁到统计卡片。");
  const next = await appList.boundingBox();
  const cardsBottom = cards.y + cards.height;
  return { x: main.x, y: 0, width: main.width, height: (cardsBottom + next.y) / 2 };
}

/**
 * 不要侧栏，只截 shot.sections 里第一个到最后一个设置块之间的范围，上下各留一点边距。
 * sections 里写的是设置块标题在界面文案里的路径，如 "aiSettings.filter.sectionTitle"。
 */
async function sectionsClip(page, shot, labels) {
  await page
    .locator("aside")
    .first()
    .evaluate((aside) => {
      aside.style.visibility = "hidden";
    });
  const main = await page.locator("main").boundingBox();
  const box = async (path) =>
    page
      .getByRole("heading", {
        name: path.split(".").reduce((node, key) => node[key], labels),
        exact: true,
      })
      .locator("xpath=ancestor::section[1]")
      .boundingBox();
  const first = await box(shot.sections[0]);
  const last = await box(shot.sections[shot.sections.length - 1]);
  const pad = 12;
  const top = first.y - pad;
  return { x: main.x, y: top, width: main.width, height: last.y + last.height + pad - top };
}

async function main() {
  const args = await options();
  if (!args) return;
  for (const doc of new Set(args.shots.map((shot) => shot.doc))) {
    await mkdir(path.join(args.out, doc), { recursive: true });
  }
  await withDemo(args, async (demo) => {
    for (const shot of args.shots) {
      const clip = shot.toCards
        ? (page) => cardsClip(page, shot, demo.labels)
        : shot.sections
          ? (page) => sectionsClip(page, shot, demo.labels)
          : undefined;
      await shoot(demo, shot, { output: path.join(args.out, shot.doc, `${shot.file}.png`), clip });
    }
  });
}

main().catch((error) => {
  console.error(error.message);
  process.exitCode = 1;
});
