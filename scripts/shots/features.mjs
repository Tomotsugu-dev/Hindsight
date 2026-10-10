#!/usr/bin/env node
/**
 * 给 docs/features/zh 下的功能文档截图，每份文档的图存到 docs/features/zh/imgs/<文档名>/。
 *
 * npm run shots:features
 * npm run shots:features -- --only month-pie,categories --theme dark
 * npm run shots:features -- --only search-results,search-preview
 *
 * 截的是网页 demo，用它自带的演示数据；按 macOS 窗口画（`?platform=macos`，带红绿灯），
 * 尺寸取 src-tauri/tauri.macos.conf.json 里的原生窗口尺寸。要截哪几张见 SHOTS。
 */

import { mkdir, readFile } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { parseArgs } from "node:util";
import { chromium } from "playwright";
import { build, preview } from "vite";

const repo = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../..");
const configFile = path.join(repo, "web/vite.config.ts");
const scopes = { day: "today", week: "week", month: "month", history: "history" };
const languages = new Set(["zh-CN", "zh-TW", "en", "ja", "pt-BR", "es"]);

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

  const wanted = values.only?.split(",").map((name) => name.trim());
  const unknown = wanted?.filter((name) => !SHOTS.some((s) => s.file === name)) ?? [];
  if (unknown.length) throw new Error(`没有这几张图：${unknown.join("、")}。`);
  const shots = wanted ? SHOTS.filter((s) => wanted.includes(s.file)) : SHOTS;
  if (!languages.has(values.lang)) throw new Error("不支持的界面语言。");
  if (!["minimal", "default", "dark"].includes(values.theme)) {
    throw new Error("--theme 只支持 minimal、default、dark。");
  }

  const date =
    values.date ??
    new Intl.DateTimeFormat("en-CA", {
      timeZone: "Asia/Tokyo",
      year: "numeric",
      month: "2-digit",
      day: "2-digit",
    }).format(new Date());
  const fixedTime = new Date(`${date}T12:00:00+09:00`);
  if (
    !/^\d{4}-\d{2}-\d{2}$/.test(date) ||
    !Number.isFinite(fixedTime.getTime()) ||
    fixedTime.toISOString().slice(0, 10) !== date
  ) {
    throw new Error("--date 必须是有效的 YYYY-MM-DD 日期。");
  }

  // 跟 macOS 原生窗口一样大，截出来的比例就是用户看到的样子。
  const tauri = JSON.parse(
    await readFile(path.join(repo, "src-tauri/tauri.macos.conf.json"), "utf8"),
  );
  const { width, height } = tauri.app.windows[0];

  return { ...values, shots, width, height, fixedTime, out: path.resolve(repo, values.out) };
}

async function launchBrowser() {
  try {
    return await chromium.launch({ channel: "chrome", headless: true });
  } catch (chromeError) {
    try {
      return await chromium.launch({ headless: true });
    } catch {
      throw new Error(
        "无法启动浏览器。请安装 Google Chrome，或执行 npx playwright install chromium。",
        { cause: chromeError },
      );
    }
  }
}

/** 关掉动画和输入光标，免得截图落在动画中间。页面重新载入后要再加一次。 */
async function disableAnimations(page) {
  await page.addStyleTag({
    content: `
      *, *::before, *::after {
        animation: none !important;
        transition: none !important;
        caret-color: transparent !important;
      }
    `,
  });
}

/**
 * 「分类」页：点侧栏进去，默认是「管理分类」；tab 为 apps / sites 时再切到「归类应用」/「归类网站」。
 * 等页面里该有的内容出来（= 数据拉完）：管理分类等「编程」这个小类，归类应用等 Notion，
 * 归类网站等 github.com。
 */
async function openCategories(page, shot, labels) {
  await disableAnimations(page);
  await page.getByRole("link", { name: labels.nav.items.categories, exact: true }).click();
  await page
    .getByRole("heading", { level: 1, name: labels.categories.title, exact: true })
    .waitFor();
  const tabs = {
    apps: { name: labels.categories.tabs.apps, ready: "Notion" },
    sites: { name: labels.categories.tabs.sites, ready: "github.com" },
  };
  const tab = tabs[shot.tab];
  if (shot.tab === "apps") {
    // 「归类应用」默认只列未指派的应用；截图要看全部，才看得到跨设备合并的那几行。
    // 筛选在这个标签挂载时从 localStorage 读，所以要在点进去之前写好。
    await page.evaluate(() => {
      localStorage.setItem(
        "hindsight.apps.filter",
        JSON.stringify({
          v: 2,
          search: "",
          selectedCategoryIds: [],
          unassignedOnly: false,
          sortBy: "default",
        }),
      );
    });
  }
  if (tab) {
    await page.getByRole("link", { name: tab.name, exact: true }).click();
    await page.getByText(tab.ready, { exact: true }).first().waitFor();
    return;
  }
  await page.getByText(labels.categories.defaults.code, { exact: true }).first().waitFor();
}

/** 全部历史页：点侧栏进去，切到年表 / 月表，等下面应用排行出来（= 范围数据拉完）。 */
async function openHistory(page, shot, labels) {
  await disableAnimations(page);
  await page.getByRole("link", { name: labels.nav.items.history, exact: true }).click();
  await page.getByRole("heading", { level: 1, name: labels.history.title, exact: true }).waitFor();
  const tabName = labels.history.chartTabs[shot.historyTab ?? "year"];
  const tab = page.getByRole("tab", { name: tabName, exact: true });
  await tab.click();
  await page.getByRole("tab", { name: tabName, exact: true, selected: true }).waitFor();
  const ranking = page.getByRole("heading", { name: labels.history.apps, exact: true });
  await ranking.locator("xpath=ancestor::section[1]").locator("ol > li").first().waitFor();
}

/**
 * 搜索页：用 demo 已有的 OCR 文字搜索，等待结果、缩略图和截图预览加载完成。
 * resultTitle 指定要展示的结果，preview 为 true 时打开它的截图预览。
 */
async function openSearch(page, shot, labels) {
  await disableAnimations(page);
  await page.getByRole("link", { name: labels.nav.items.history, exact: true }).click();
  await page.getByRole("link", { name: labels.history.tabs.search, exact: true }).click();
  await page.getByPlaceholder(labels.search.placeholder, { exact: true }).fill(shot.query);
  const result = page.getByRole("button").filter({
    has: page.getByText(`Google Chrome · ${shot.resultTitle}`, { exact: true }),
  });
  await result.waitFor({ state: "visible" });
  // 列表缩略图使用 lazy loading，等结果里的真实图片加载后再截图。
  await page.waitForFunction(() => {
    const images = [...document.querySelectorAll("main button img")];
    return images.length > 0 && images.every((img) => img.complete && img.naturalWidth > 0);
  });
  if (!shot.preview) return;
  await result.click();
  const image = page.getByRole("img", { name: labels.search.viewerAlt, exact: true });
  await image.waitFor({ state: "visible" });
  await page.waitForFunction(
    (alt) => {
      const img = [...document.images].find((item) => item.alt === alt);
      return img?.complete && img.naturalWidth > 0;
    },
    labels.search.viewerAlt,
  );
  await page.getByText(labels.search.locating, { exact: false }).waitFor({ state: "hidden" });
  if (shot.highlights) {
    await image.locator("..").locator(":scope > div").first().waitFor({ state: "visible" });
  }
}

/** 打开 shot 要的那一页：先把视图和占比层写进 localStorage，重新载入后应用才会读到。 */
async function openPage(page, shot, labels) {
  const scope = scopes[shot.page];
  if (shot.page === "search") {
    await openSearch(page, shot, labels);
    return;
  }
  if (shot.page === "categories") {
    await openCategories(page, shot, labels);
    return;
  }
  if (shot.page === "history") {
    await openHistory(page, shot, labels);
    return;
  }
  await page.evaluate(
    ({ scope, view, depth }) => {
      localStorage.setItem(`hindsight.stats.view.${scope}`, view);
      localStorage.setItem(`hindsight.stats.pieDepth.${scope}`, depth);
    },
    { scope, view: shot.view, depth: shot.depth ?? "supers" },
  );
  await page.reload({ waitUntil: "networkidle" });
  await disableAnimations(page);
  await page.getByRole("link", { name: labels.nav.items[scope], exact: true }).click();
  await page.getByRole("heading", { level: 1, name: labels[scope].title, exact: true }).waitFor();
  const ranking = page.getByRole("heading", { name: labels[scope].ranks.topApps, exact: true });
  await ranking.locator("xpath=ancestor::section[1]").locator("ol > li").first().waitFor();

  const toggle = labels.today.chart.viewToggle;
  const viewLabel = shot.view === "bars" ? toggle.ariaBars : toggle.ariaPie;
  await page.getByRole("button", { name: viewLabel, exact: true, pressed: true }).waitFor();
}

/**
 * 跟用户一样操作：先点占比视图里的大类那一行，再点底部的小类标签。
 * 左右两边上月、下月的预览卡片里也有同样的行，按钮是禁用的，所以只找能点的。
 */
async function pick(page, { superId, catId }, labels) {
  const superName = labels.categories.super.defaults[superId];
  const row = page
    .locator("li > button:not([disabled])")
    .filter({ has: page.getByText(superName, { exact: true }) });
  await row.click();
  await page
    .locator("li > button[data-active='true']:not([disabled])")
    .filter({ has: page.getByText(superName, { exact: true }) })
    .waitFor();
  if (!catId) return;
  const catName = labels.categories.defaults[catId];
  await page
    .locator("button[aria-pressed]:not([disabled])")
    .filter({ has: page.getByText(catName, { exact: true }) })
    .click();
  await page.waitForFunction(
    (name) =>
      [...document.querySelectorAll("button[aria-pressed='true']")].some((button) =>
        button.textContent?.includes(name),
      ),
    catName,
  );
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

async function main() {
  const args = await options();
  if (!args) return;
  const labels = JSON.parse(
    await readFile(path.join(repo, `src/i18n/locales/${args.lang}.json`), "utf8"),
  );
  for (const doc of new Set(args.shots.map((shot) => shot.doc))) {
    await mkdir(path.join(args.out, doc), { recursive: true });
  }
  console.log("构建网页 demo…");
  await build({ configFile, logLevel: "warn" });

  let server;
  let browser;
  try {
    server = await preview({
      configFile,
      logLevel: "warn",
      preview: { host: "127.0.0.1", port: 0, open: false },
    });
    const address = server.httpServer.address();
    if (!address || typeof address === "string") throw new Error("本地预览启动失败。");
    browser = await launchBrowser();
    const context = await browser.newContext({
      viewport: { width: args.width, height: args.height },
      deviceScaleFactor: 2,
      locale: args.lang,
      timezoneId: "Asia/Tokyo",
      colorScheme: args.theme === "dark" ? "dark" : "light",
      reducedMotion: "reduce",
    });
    await context.addInitScript(
      ({ lang, theme }) => {
        localStorage.setItem("hindsight.locale", lang);
        localStorage.setItem("hindsight.theme", theme);
        // 使用应用已有的无动画回退，避免截图落在切换动画中间。
        Object.defineProperty(document, "startViewTransition", {
          configurable: true,
          value: undefined,
        });
      },
      { lang: args.lang, theme: args.theme },
    );
    const page = await context.newPage();
    page.setDefaultTimeout(20_000);
    const errors = [];
    page.on("pageerror", (error) => errors.push(error.message));
    await page.clock.setFixedTime(args.fixedTime);
    await page.goto(`http://127.0.0.1:${address.port}/demo/?lang=${args.lang}&platform=macos`, {
      waitUntil: "networkidle",
    });

    for (const shot of args.shots) {
      await openPage(page, shot, labels);
      if (shot.pick) await pick(page, shot.pick, labels);
      await page.waitForFunction(
        () =>
          document.fonts.status === "loaded" &&
          [...document.images].every((image) => image.complete),
      );
      await page.locator("main").evaluate((element) => {
        element.scrollTop = 0;
      });
      // 鼠标移到角落，免得停在某一行上触发悬停效果。
      await page.mouse.move(args.width - 1, args.height - 1);
      // 字体、窗口尺寸及圆环完成布局后再截图。
      await page.evaluate(
        () => new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve))),
      );
      if (errors.length) throw new Error(`页面运行错误：${errors.join("；")}`);
      const output = path.join(args.out, shot.doc, `${shot.file}.png`);
      const clip = shot.toCards
        ? await cardsClip(page, shot, labels)
        : { x: 0, y: 0, width: args.width, height: args.height };
      await page.screenshot({ path: output, animations: "disabled", clip });
      console.log(
        `已保存 ${path.relative(repo, output)}（${Math.round(clip.width * 2)} × ${Math.round(clip.height * 2)}）`,
      );
    }
  } finally {
    try {
      await browser?.close();
    } finally {
      if (server) await new Promise((resolve) => server.httpServer.close(resolve));
    }
  }
}

main().catch((error) => {
  console.error(error.message);
  process.exitCode = 1;
});
