// 把整站组装到 dist-web/，由 Cloudflare Workers 静态资源发布（见 web/wrangler.jsonc）。
// demo 和文档站先由 build:demo、build:docs 生成到 web/ 里（见 package.json 的 build:web），
// 这里只负责拼装：
//   web/ 下的页面、样式和构建产物（不含 demo-src、配置、README 这些源文件）
//   页面引用的 /assets/video/* → 从 docs/intro/<lang>/shot_video/ 取
// 拼完校验所有站内 src / href / poster 都指向存在的文件。
import { cpSync, existsSync, mkdirSync, readdirSync, readFileSync, rmSync, statSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../..");
const WEB = path.join(ROOT, "web");
const OUT = path.join(ROOT, "dist-web");
const INTRO = path.join(ROOT, "docs/intro");

// web/ 里只在构建时用、不该发布出去的文件
const SOURCE_ONLY = new Set(["demo-src", "tsconfig.json", "vite.config.ts", "wrangler.jsonc"]);
const published = (src) => {
  const name = path.basename(src);
  return !name.startsWith(".") && !name.startsWith("README") && !SOURCE_ONLY.has(name);
};

// Workers 静态资源单个文件上限 25 MiB
const MAX_FILE = 25 * 1024 * 1024;

const walk = (dir) =>
  readdirSync(dir, { withFileTypes: true }).flatMap((e) =>
    e.isDirectory() ? walk(path.join(dir, e.name)) : [path.join(dir, e.name)],
  );

function build() {
  for (const required of ["demo/index.html", "docs/index.html"]) {
    if (!existsSync(path.join(WEB, required))) {
      console.error(`缺少 web/${required}，先跑 npm run build:demo 和 npm run build:docs`);
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
