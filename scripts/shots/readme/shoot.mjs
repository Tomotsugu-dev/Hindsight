#!/usr/bin/env node
/**
 * 给 README 截介绍图，所有语言一次生成，存到 docs/intro/<语言>/imgs/，
 * 文件名就是 README 里引用的那几个。
 *
 * npm run shots:readme
 * npm run shots:readme -- --lang ja
 * npm run shots:readme -- --only daily,cloud_sync
 *
 * 构建 demo、打开页面这些步骤在 ../web-demo.mjs，和 features.mjs 共用。要截哪几张见 SHOTS。
 */

import { mkdir } from "node:fs/promises";
import path from "node:path";
import { parseArgs } from "node:util";
import {
  CHAT_QUESTIONS,
  demoOptions,
  repo,
  selectShots,
  shoot,
  tokyoToday,
  withDemoPage,
  withDemoSession,
} from "../web-demo.mjs";

// 每种界面语言的 README 用哪个目录的图。
const imgDirs = { "zh-CN": "zh", "zh-TW": "zh-TW", en: "en", ja: "ja", "pt-BR": "pt", es: "es" };

/**
 * README 里的图，都截整个窗口。page、view 的意思和 features.mjs 的 SHOTS 一样；
 * detail：再点开「最常用应用」排行第一的应用，截它的详情抽屉。
 */
const SHOTS = [
  { file: "daily", page: "day", view: "bars" },
  { file: "app_detail", page: "day", view: "bars", detail: true },
  { file: "weekly", page: "week", view: "bars" },
  { file: "monthly", page: "month", view: "bars" },
  { file: "monthly_cal", page: "month", view: "pie" },
  { file: "ai_summary", page: "aiSummary", ready: "HourlyChart" },
  { file: "ai_chatbot", page: "chat", questions: CHAT_QUESTIONS },
  { file: "cloud_sync", page: "cloudSync" },
];

/** 东京最近一个周日（今天是周日就是今天）：周统计只有今天以前有数据，这天七天都满。 */
function lastSunday() {
  const day = new Date(`${tokyoToday()}T00:00:00Z`);
  day.setUTCDate(day.getUTCDate() - day.getUTCDay());
  return day.toISOString().slice(0, 10);
}

async function options() {
  const { values } = parseArgs({
    options: {
      only: { type: "string" },
      out: { type: "string", default: "docs/intro" },
      lang: { type: "string" },
      theme: { type: "string", default: "minimal" },
      date: { type: "string" },
      help: { type: "boolean", short: "h" },
    },
  });
  if (values.help) {
    console.log(`用法：npm run shots:readme -- [选项]
  --only NAME,NAME               只截这几张，默认全截：${SHOTS.map((s) => s.file).join("、")}
  --lang LOCALE,LOCALE           只截这几种界面语言，默认全部：${Object.keys(imgDirs).join("、")}
  --out PATH                     输出根目录，默认 docs/intro，图存到 <语言目录>/imgs
  --theme minimal|default|dark   外观，默认 minimal
  --date YYYY-MM-DD              演示数据的「今天」，默认东京最近一个周日

窗口尺寸取 src-tauri/tauri.macos.conf.json。需要 Google Chrome，
或先执行 npx playwright install chromium。`);
    return null;
  }

  const langs = values.lang ? values.lang.split(",") : Object.keys(imgDirs);
  for (const lang of langs) {
    if (!imgDirs[lang]) {
      throw new Error(`--lang 只支持 ${Object.keys(imgDirs).join("、")}。`);
    }
  }
  const shots = selectShots(SHOTS, values.only);
  const date = values.date ?? lastSunday();
  const demos = [];
  for (const lang of langs) demos.push(await demoOptions({ lang, theme: values.theme, date }));
  return { demos, shots, out: path.resolve(repo, values.out) };
}

async function main() {
  const args = await options();
  if (!args) return;
  await withDemoSession(async (session) => {
    for (const opts of args.demos) {
      const dir = path.join(args.out, imgDirs[opts.lang], "imgs");
      await mkdir(dir, { recursive: true });
      await withDemoPage(session, opts, async (demo) => {
        for (const shot of args.shots) {
          await shoot(demo, shot, { output: path.join(dir, `${shot.file}.png`) });
        }
      });
    }
  });
}

main().catch((error) => {
  console.error(error.message);
  process.exitCode = 1;
});
