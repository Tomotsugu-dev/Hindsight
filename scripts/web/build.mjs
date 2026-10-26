// Assemble the public site in dist-web/ for the Worker in web/wrangler.jsonc.
// build:demo and build:docs generate their web/ outputs before this script runs.
// Copy public pages and assets, plus referenced videos from docs/intro/.
// Validate that every local src, href, and poster points to an existing file.
import { cpSync, existsSync, mkdirSync, readdirSync, readFileSync, rmSync, statSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../..");
const WEB = path.join(ROOT, "web");
const OUT = path.join(ROOT, "dist-web");
const INTRO = path.join(ROOT, "docs/intro");

// Exclude source-only files.
const SOURCE_ONLY = new Set([
  "demo-src",
  "themes-src",
  "tsconfig.json",
  "vite.config.ts",
  "wrangler.jsonc",
  "worker.js",
  "themes-data.js",
]);
// 测试只在 CI 里跑，不发布
const TEST_FILE = /\.test\.[jt]s$/;
const published = (src) => {
  const name = path.basename(src);
  return (
    !name.startsWith(".") && !name.startsWith("README") && !SOURCE_ONLY.has(name) && !TEST_FILE.test(name)
  );
};

// Workers 静态资源单个文件上限 25 MiB
const MAX_FILE = 25 * 1024 * 1024;

const walk = (dir) =>
  readdirSync(dir, { withFileTypes: true }).flatMap((e) =>
    e.isDirectory() ? walk(path.join(dir, e.name)) : [path.join(dir, e.name)],
  );

function build() {
  for (const required of ["demo/index.html", "themes/index.html", "docs/index.html"]) {
    if (!existsSync(path.join(WEB, required))) {
      console.error(`缺少 web/${required}，先跑 npm run build:demo、build:themes 和 build:docs`);
      process.exit(1);
    }
  }
  rmSync(OUT, { recursive: true, force: true });
  cpSync(WEB, OUT, { recursive: true, filter: published });

  const pages = walk(OUT).filter((f) => f.endsWith(".html"));
  copyVideos(pages);
  check(pages);

  const files = walk(OUT);
  const bytes = files.reduce((n, f) => n + statSync(f).size, 0);
  console.log(`站点已组装：${files.length} 个文件，${(bytes / 1024 / 1024).toFixed(1)} MB → ${path.relative(ROOT, OUT)}`);
}

// 视频和封面按文件名从 docs/intro/*/shot_video/ 里找，只拷页面实际引用的
function copyVideos(pages) {
  const sources = new Map(
    readdirSync(INTRO).flatMap((lang) => {
      const dir = path.join(INTRO, lang, "shot_video");
      return existsSync(dir) ? readdirSync(dir).map((name) => [name, path.join(dir, name)]) : [];
    }),
  );
  const wanted = new Set(
    pages.flatMap((f) => [...readFileSync(f, "utf8").matchAll(/"\/assets\/video\/([^"?#]+)"/g)].map((m) => m[1])),
  );
  mkdirSync(path.join(OUT, "assets/video"), { recursive: true });
  for (const name of wanted) {
    // 找不到的留给 check() 统一报错
    if (sources.has(name)) cpSync(sources.get(name), path.join(OUT, "assets/video", name));
  }
}

// 构建期校验：站内引用的文件必须存在，且不超过 Pages 的单文件上限
function check(pages) {
  const errors = [];
  for (const file of pages) {
    const page = "/" + path.relative(OUT, file);
    for (const [, url] of readFileSync(file, "utf8").matchAll(/(?:src|href|poster)="(\/(?!\/)[^"]*)"/g)) {
      const p = decodeURIComponent(url.split(/[?#]/)[0]);
      const target = path.join(OUT, p.endsWith("/") ? `${p}index.html` : p);
      if (!existsSync(target)) errors.push(`${page}: 文件不存在 ${url}`);
    }
  }
  for (const file of walk(OUT)) {
    const size = statSync(file).size;
    if (size > MAX_FILE) errors.push(`${path.relative(OUT, file)}: ${(size / 1024 / 1024).toFixed(1)} MB，超过单文件 25 MiB 上限`);
  }
  if (errors.length) {
    console.error([...new Set(errors)].join("\n"));
    process.exit(1);
  }
}

build();
