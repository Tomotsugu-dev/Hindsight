/**
 * 截图脚本共用的部分：构建网页 demo、启动本地预览、打开浏览器，以及打开各个页面的步骤。
 * features.mjs（功能文档）和 readme/shoot.mjs（README 介绍图）都用它。
 *
 * 截的是网页 demo，用它自带的演示数据；按 macOS 窗口画（`?platform=macos`，带红绿灯），
 * 尺寸取 src-tauri/tauri.macos.conf.json 里的原生窗口尺寸，以 2 倍像素保存。
 */

import { readFile } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { chromium } from "playwright";
import { build, preview } from "vite";

export const repo = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../..");
const configFile = path.join(repo, "web/vite.config.ts");
export const scopes = { day: "today", week: "week", month: "month", history: "history" };
const languages = new Set(["zh-CN", "zh-TW", "en", "ja", "pt-BR", "es"]);

// 「对话」截图问的问题。每种语言的问题要和 web/demo-src/api-mock.ts 里的
// DEMO_PROJECT_ANSWERS 一致，demo 才会给出像样的回答。
export const CHAT_QUESTIONS = {
  "zh-CN": "这周我在 Hindsight 项目上花了多少时间？",
  "zh-TW": "這週我在 Hindsight 專案上花了多少時間？",
  en: "How much time did I spend on the Hindsight project this week?",
  ja: "今週、Hindsight プロジェクトにどのくらい時間を使いましたか？",
  "pt-BR": "Quanto tempo passei no projeto Hindsight esta semana?",
  es: "¿Cuánto tiempo dediqué al proyecto Hindsight esta semana?",
};

/** 按 --only 选出要截的图，默认全部；有对不上的名字就报错。 */
export function selectShots(shots, only) {
  const wanted = only?.split(",").map((name) => name.trim());
  const unknown = wanted?.filter((name) => !shots.some((s) => s.file === name)) ?? [];
  if (unknown.length) throw new Error(`没有这几张图：${unknown.join("、")}。`);
  return wanted ? shots.filter((s) => wanted.includes(s.file)) : shots;
}

/** 东京现在是哪一天，YYYY-MM-DD。 */
export function tokyoToday() {
  return new Intl.DateTimeFormat("en-CA", {
    timeZone: "Asia/Tokyo",
    year: "numeric",
    month: "2-digit",
    day: "2-digit",
  }).format(new Date());
}

/**
 * 校验 --lang、--theme、--date，读出窗口尺寸。
 * date 是演示数据的「今天」，不给就用东京当天；截图时页面时钟固定在这天中午。
 */
export async function demoOptions({ lang, theme, date }) {
  if (!languages.has(lang)) throw new Error("不支持的界面语言。");
  if (!["minimal", "default", "dark"].includes(theme)) {
    throw new Error("--theme 只支持 minimal、default、dark。");
  }

  const day = date ?? tokyoToday();
  const fixedTime = new Date(`${day}T12:00:00+09:00`);
  if (
    !/^\d{4}-\d{2}-\d{2}$/.test(day) ||
    !Number.isFinite(fixedTime.getTime()) ||
    fixedTime.toISOString().slice(0, 10) !== day
  ) {
    throw new Error("--date 必须是有效的 YYYY-MM-DD 日期。");
  }

  // 跟 macOS 原生窗口一样大，截出来的比例就是用户看到的样子。
  const tauri = JSON.parse(
    await readFile(path.join(repo, "src-tauri/tauri.macos.conf.json"), "utf8"),
  );
  const { width, height } = tauri.app.windows[0];

  return { lang, theme, fixedTime, width, height };
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

/** 「AI 总结」页：默认就是日报；等报告里 shot.ready 这段文字出来（= 分时段内容渲染完）。 */
async function openAiSummary(page, shot, labels) {
  await disableAnimations(page);
  await page.getByRole("link", { name: labels.nav.items.aiSummary, exact: true }).click();
  await page
    .getByRole("heading", { level: 1, name: labels.aiSummary.title, exact: true })
    .waitFor();
  await page.getByText(shot.ready, { exact: false }).first().waitFor();
}

/** 「AI 设置」页：shot.tab 为 general 时切到「常规」标签，等「分析这些分类」那一行出来。 */
async function openAiSettings(page, shot, labels) {
  await disableAnimations(page);
  await page.getByRole("link", { name: labels.nav.items.aiSettings, exact: true }).click();
  await page
    .getByRole("heading", { level: 1, name: labels.aiSettings.title, exact: true })
    .waitFor();
  if (shot.tab === "general") {
    await page.getByRole("link", { name: labels.aiSettings.tabs.general, exact: true }).click();
    await page.getByText(labels.aiSettings.filter.rowLabel, { exact: true }).waitFor();
  }
}

/**
 * 「对话」页：把 shot.questions 里当前语言的问题填进输入框发送，等回答里第一条来源出现再截图。
 */
async function openChat(page, shot, labels, lang) {
  await disableAnimations(page);
  await page.getByRole("link", { name: labels.nav.items.chat, exact: true }).click();
  const input = page.getByPlaceholder(labels.chat.input.placeholder, { exact: true });
  await input.fill(shot.questions[lang]);
  await input.press("Enter");
  const source = page.getByText("Visual Studio Code · ", { exact: false }).first();
  await source.waitFor();
  // 回答出来后消息列表会滚到最底，提问被顶到上面；滚回去，提问和回答一起入镜。
  await source.evaluate((element) => {
    for (let node = element.parentElement; node; node = node.parentElement) {
      if (node.scrollHeight > node.clientHeight && getComputedStyle(node).overflowY !== "visible") {
        node.scrollTop = 0;
        return;
      }
    }
  });
}

/**
 * 「云同步」页：demo 一开始没登录，页面会自己展开「选择同步服务」。跟用户一样，先点 Google
 * 卡片上的「配置 OAuth」，填一组假的凭证，再点「使用 Google 登录」；demo 模拟登录成功，
 * 账号是 demo@hindsight.app。等「已连接到 Google Drive」出来再截图。
 *
 * 侧栏的登录状态只在窗口获得焦点时和每分钟刷新一次。用户从浏览器登录完会切回应用，
 * 这里补发一次焦点事件，等侧栏不再显示「未登录」。
 */
async function openCloudSync(page, shot, labels) {
  await disableAnimations(page);
  await page.getByRole("link", { name: labels.nav.items.devices, exact: true }).click();
  await page.getByRole("heading", { level: 1, name: labels.devices.title, exact: true }).waitFor();
  const cloud = labels.devices.cloud;
  await page.getByRole("button", { name: cloud.actions.configureOAuth, exact: true }).click();
  await page
    .getByPlaceholder("xxxxxx.apps.googleusercontent.com", { exact: true })
    .fill("demo.apps.googleusercontent.com");
  await page.getByPlaceholder("GOCSPX-...", { exact: true }).fill("GOCSPX-demo");
  await page.getByRole("button", { name: cloud.actions.signInWithGoogle, exact: true }).click();
  const connected = cloud.state.connectedTo.replace("{{name}}", cloud.services.googleDrive.name);
  await page.getByText(connected, { exact: true }).waitFor();
  await page.evaluate(() => window.dispatchEvent(new Event("focus")));
  await page
    .locator("aside")
    .first()
    .getByText(labels.sidebar.sync.signedOut, { exact: true })
    .waitFor({ state: "detached" });
}

/** 点开「最常用应用」排行第一的应用，等详情抽屉里的窗口标题列出来。 */
async function openAppDetail(page, ranking) {
  await ranking
    .locator("xpath=ancestor::section[1]")
    .locator("ol > li")
    .first()
    .locator("button")
    .first()
    .click();
  await page.getByRole("dialog").locator("li").first().waitFor();
}

/** 打开 shot 要的那一页：先把视图和占比层写进 localStorage，重新载入后应用才会读到。 */
async function openPage(page, shot, labels, lang) {
  const scope = scopes[shot.page];
  if (shot.page === "search") {
    await openSearch(page, shot, labels);
    return;
  }
  if (shot.page === "aiSummary") {
    await openAiSummary(page, shot, labels);
    return;
  }
  if (shot.page === "aiSettings") {
    await openAiSettings(page, shot, labels);
    return;
  }
  if (shot.page === "chat") {
    await openChat(page, shot, labels, lang);
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
  if (shot.page === "cloudSync") {
    await openCloudSync(page, shot, labels);
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
  if (shot.detail) await openAppDetail(page, ranking);
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
 * 构建 demo、启动本地预览和浏览器，跑 fn；跑完关掉浏览器和预览服务。
 * fn 拿到的 session 交给 withDemoPage，按语言打开页面。一次构建可以截多种语言。
 */
export async function withDemoSession(fn) {
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
    await fn({ browser, port: address.port });
  } finally {
    try {
      await browser?.close();
    } finally {
      if (server) await new Promise((resolve) => server.httpServer.close(resolve));
    }
  }
}

/**
 * 在 session 里按 opts（demoOptions 的结果）打开 demo 页面，跑 fn；跑完关掉这个页面。
 * fn 拿到的对象交给 shoot 截图。
 */
export async function withDemoPage(session, opts, fn) {
  const labels = JSON.parse(
    await readFile(path.join(repo, `src/i18n/locales/${opts.lang}.json`), "utf8"),
  );
  const context = await session.browser.newContext({
    viewport: { width: opts.width, height: opts.height },
    deviceScaleFactor: 2,
    locale: opts.lang,
    timezoneId: "Asia/Tokyo",
    colorScheme: opts.theme === "dark" ? "dark" : "light",
    reducedMotion: "reduce",
  });
  try {
    await context.addInitScript(
      ({ lang, theme }) => {
        localStorage.setItem("hindsight.locale", lang);
        // 只在还没存过时写：每张图要换主题时由 shoot 改，重新载入后不能被覆盖回来。
        if (localStorage.getItem("hindsight.theme") === null) {
          localStorage.setItem("hindsight.theme", theme);
        }
        // 使用应用已有的无动画回退，避免截图落在切换动画中间。
        Object.defineProperty(document, "startViewTransition", {
          configurable: true,
          value: undefined,
        });
      },
      { lang: opts.lang, theme: opts.theme },
    );
    const page = await context.newPage();
    page.setDefaultTimeout(20_000);
    const errors = [];
    page.on("pageerror", (error) => errors.push(error.message));
    await page.clock.setFixedTime(opts.fixedTime);
    await page.goto(`http://127.0.0.1:${session.port}/demo/?lang=${opts.lang}&platform=macos`, {
      waitUntil: "networkidle",
    });
    await fn({ ...opts, page, labels, errors });
  } finally {
    await context.close();
  }
}

/** 只截一种语言时用：构建、打开页面、跑 fn 一步到位。 */
export async function withDemo(opts, fn) {
  await withDemoSession((session) => withDemoPage(session, opts, fn));
}

/**
 * 截一张图存到 output。clip 是可选的回调，在页面准备好后返回裁剪范围；不给就截整个窗口。
 *
 * 每张图都从重新载入的页面开始，上一张图留下的弹窗、选中状态不会带进来。
 * shot.theme 指定这张图用的主题，没指定就用命令行的 --theme；
 * 主题是页面载入时读的，所以先写进存储再载入。
 */
export async function shoot(demo, shot, { output, clip }) {
  const { page, labels, errors, lang, width, height } = demo;
  await page.evaluate(
    (value) => localStorage.setItem("hindsight.theme", value),
    shot.theme ?? demo.theme,
  );
  await page.reload({ waitUntil: "networkidle" });
  await openPage(page, shot, labels, lang);
  if (shot.pick) await pick(page, shot.pick, labels);
  await page.waitForFunction(
    () =>
      document.fonts.status === "loaded" &&
      [...document.images].every((image) => image.complete),
  );
  await page.locator("main").evaluate((element, toEnd) => {
    element.scrollTop = toEnd ? element.scrollHeight : 0;
  }, Boolean(shot.scrollToEnd));
  // 鼠标移到角落，免得停在某一行上触发悬停效果。详情抽屉占着右边，就移到左下角。
  await page.mouse.move(shot.detail ? 1 : width - 1, height - 1);
  // 字体、窗口尺寸及圆环完成布局后再截图。
  await page.evaluate(
    () => new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve))),
  );
  if (errors.length) throw new Error(`页面运行错误：${errors.join("；")}`);
  const area = (await clip?.(page)) ?? { x: 0, y: 0, width, height };
  await page.screenshot({ path: output, animations: "disabled", clip: area });
  // 裁剪时藏起来的侧栏要还原，下一张图还要靠它切页面。
  await page
    .locator("aside")
    .first()
    .evaluate((aside) => {
      aside.style.visibility = "";
    });
  console.log(
    `已保存 ${path.relative(repo, output)}（${Math.round(area.width * 2)} × ${Math.round(area.height * 2)}）`,
  );
}
