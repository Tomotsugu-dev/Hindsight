// 把 docs/features/<lang>/*.md 渲染成静态 HTML，输出到 web/docs/。
// 搜索索引由随后的 pagefind 生成（见 package.json 的 build:docs）。
import { cpSync, existsSync, mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import GithubSlugger from "github-slugger";
import MarkdownIt from "markdown-it";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../..");
const SRC = path.join(ROOT, "docs/features");
const OUT = path.join(ROOT, "web/docs");

// dir: 源目录名；slug: 站点 URL 段（与 landing 页目录一致）；landing: 该语言官网首页
const LANGS = [
  { dir: "zh", slug: "zh", html: "zh-CN", name: "简体中文", docs: "功能", landing: "/" },
  { dir: "zh-TW", slug: "tw", html: "zh-TW", name: "繁體中文", docs: "功能", landing: "/tw/" },
  { dir: "en", slug: "en", html: "en", name: "English", docs: "Features", landing: "/en/" },
  { dir: "ja", slug: "ja", html: "ja", name: "日本語", docs: "機能", landing: "/ja/" },
  { dir: "es", slug: "es", html: "es", name: "Español", docs: "Funciones", landing: "/en/" },
  { dir: "pt-BR", slug: "pt", html: "pt-BR", name: "Português", docs: "Recursos", landing: "/pt/" },
];

const escapeHtml = (s) =>
  s.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;").replace(/"/g, "&quot;");

function createMarkdown() {
  const md = new MarkdownIt({ html: true });
  // 标题 id 与 GitHub 的锚点算法一致，文档里写给 GitHub 的 #锚点 链接无需改动
  md.core.ruler.push("heading_ids", (state) => {
    const slugger = new GithubSlugger();
    const tokens = state.tokens;
    tokens.forEach((token, i) => {
      if (token.type !== "heading_open") return;
      const text = tokens[i + 1].children
        .filter((t) => t.type === "text" || t.type === "code_inline")
        .map((t) => t.content)
        .join("");
      token.attrSet("id", slugger.slug(text));
    });
  });
  return md;
}

// 页面顺序以 README 目录表里的出现顺序为准
function pageOrder(readme) {
  return [...readme.matchAll(/\]\(\.\/([a-z-]+)\.md\)/g)].map((m) => m[1]);
}

function pageTitle(md, source, isIndex) {
  if (isIndex) {
    const m = source.match(/<h1[^>]*>(.*?)<\/h1>/s);
    return m[1].trim();
  }
  const tokens = md.parse(source, {});
  const i = tokens.findIndex((t) => t.type === "heading_open" && t.tag === "h1");
  return tokens[i + 1].content;
}

function rewriteLinks(html, slug) {
  return html
    .replace(/(href=")\.\/([a-z-]+)\.md(#[^"]*)?"/g, (_, h, page, hash = "") => `${h}/docs/${slug}/${page}/${hash}"`)
    .replace(/(src=")\.\/imgs\//g, `$1/docs/${slug}/imgs/`)
    .replace(/src="(?:\.\.\/)+src\/assets\/logo\.png"/g, 'src="/docs/assets/logo.png"')
    .replace(/<img /g, '<img loading="lazy" ');
}

function renderPage({ lang, key, title, body, pages, isIndex }) {
  const url = (k) => `/docs/${lang.slug}/${k ? `${k}/` : ""}`;
  const side = [{ key: "", title: pages.index }, ...pages.list]
    .map((p) => `<a href="${url(p.key)}"${p.key === key ? ' aria-current="page"' : ""}>${escapeHtml(p.title)}</a>`)
    .join("\n        ");
  const langItems = LANGS.map((l) =>
    l === lang
      ? `<li><span class="lang-active">${l.name}</span></li>`
      : `<li><a href="/docs/${l.slug}/${key ? `${key}/` : ""}" hreflang="${l.html}">${l.name}</a></li>`,
  ).join("\n            ");
  return `<!DOCTYPE html>
<html lang="${lang.html}">
<head>
  <meta charset="UTF-8">
  <meta name="viewport" content="width=device-width, initial-scale=1.0">
  <title>${escapeHtml(title.replace(/<[^>]+>/g, ""))} · Hindsight ${lang.docs}</title>
  <link rel="icon" type="image/png" href="/docs/assets/logo.png">
  <link rel="stylesheet" href="/styles.css?v=2">
  <link rel="stylesheet" href="/docs/pagefind/pagefind-ui.css">
  <link rel="stylesheet" href="/docs/docs.css?v=1">
</head>
<body>
  <header class="nav">
    <div class="container nav-inner">
      <a href="${lang.landing}" class="nav-brand">
        <img src="/docs/assets/logo.png" alt="" class="nav-logo">
        <span class="nav-name">Hindsight</span>
      </a>
      <a href="${url("")}" class="docs-tag">${lang.docs}</a>
      <div id="search" class="nav-search"></div>
      <div class="nav-links">
        <details class="lang-menu">
          <summary>
            <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><circle cx="12" cy="12" r="10"></circle><path d="M12 2a15.3 15.3 0 0 1 4 10 15.3 15.3 0 0 1-4 10 15.3 15.3 0 0 1-4-10 15.3 15.3 0 0 1 4-10z"></path><line x1="2" y1="12" x2="22" y2="12"></line></svg>
            <span>${lang.name}</span>
          </summary>
          <ul>
            ${langItems}
          </ul>
        </details>
        <a href="https://github.com/Tomotsugu-dev/Hindsight" class="nav-github" aria-label="GitHub">
          <svg width="20" height="20" viewBox="0 0 24 24" fill="currentColor"><path d="M12 .5C5.65.5.5 5.65.5 12c0 5.08 3.29 9.39 7.86 10.92.58.1.79-.25.79-.56 0-.28-.01-1.02-.02-2-3.2.7-3.87-1.54-3.87-1.54-.52-1.33-1.28-1.69-1.28-1.69-1.05-.72.08-.7.08-.7 1.16.08 1.77 1.19 1.77 1.19 1.03 1.77 2.7 1.26 3.36.96.1-.75.4-1.26.73-1.55-2.55-.29-5.23-1.28-5.23-5.69 0-1.26.45-2.29 1.19-3.1-.12-.29-.52-1.47.11-3.05 0 0 .97-.31 3.18 1.18a11.05 11.05 0 0 1 5.79 0c2.2-1.49 3.17-1.18 3.17-1.18.63 1.58.23 2.76.12 3.05.74.81 1.18 1.84 1.18 3.1 0 4.42-2.69 5.4-5.25 5.68.41.36.78 1.06.78 2.14 0 1.55-.01 2.79-.01 3.17 0 .31.21.67.8.55C20.21 21.39 23.5 17.08 23.5 12 23.5 5.65 18.35.5 12 .5z"/></svg>
        </a>
      </div>
    </div>
  </header>

  <div class="doc-layout">
    <nav class="doc-side" aria-label="${lang.docs}">
        ${side}
    </nav>
    <main class="doc-main">
      <article class="doc-body"${isIndex ? "" : " data-pagefind-body"}>
${body}
      </article>
      <footer class="doc-footer">© 2026 Hindsight · MIT License</footer>
    </main>
  </div>

  <script src="/docs/pagefind/pagefind-ui.js"></script>
  <script>
    new PagefindUI({ element: "#search", baseUrl: "/docs/", showImages: false, showSubResults: true });
  </script>
</body>
</html>
`;
}

// 站点根 /docs/：按浏览器语言跳到对应语言
function renderRoot() {
  const links = LANGS.map((l) => `<li><a href="/docs/${l.slug}/">${l.name}</a></li>`).join("");
  return `<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="UTF-8">
  <meta name="viewport" content="width=device-width, initial-scale=1.0">
  <title>Hindsight Features</title>
  <link rel="icon" type="image/png" href="/docs/assets/logo.png">
  <script>
    (function () {
      var lang = (navigator.language || "en").toLowerCase();
      var slug = "en";
      if (lang.indexOf("zh") === 0) slug = /^zh-?(tw|hk|mo|hant)/.test(lang) ? "tw" : "zh";
      else if (lang.indexOf("ja") === 0) slug = "ja";
      else if (lang.indexOf("es") === 0) slug = "es";
      else if (lang.indexOf("pt") === 0) slug = "pt";
      location.replace("/docs/" + slug + "/");
    })();
  </script>
</head>
<body><ul>${links}</ul></body>
</html>
`;
}

function build() {
  rmSync(OUT, { recursive: true, force: true });
  mkdirSync(path.join(OUT, "assets"), { recursive: true });
  cpSync(path.join(ROOT, "src/assets/logo.png"), path.join(OUT, "assets/logo.png"));
  cpSync(path.join(import.meta.dirname, "docs.css"), path.join(OUT, "docs.css"));
  writeFileSync(path.join(OUT, "index.html"), renderRoot());

  const md = createMarkdown();
  const written = []; // { file, slug, html }
  for (const lang of LANGS) {
    const dir = path.join(SRC, lang.dir);
    const read = (name) => readFileSync(path.join(dir, `${name}.md`), "utf8");
    const readme = read("README");
    const keys = pageOrder(readme);
    const sources = Object.fromEntries(keys.map((k) => [k, read(k)]));
    const pages = {
      index: pageTitle(md, readme, true),
      list: keys.map((k) => ({ key: k, title: pageTitle(md, sources[k], false) })),
    };
    const emit = (key, source, isIndex) => {
      const title = isIndex ? pages.index : pages.list.find((p) => p.key === key).title;
      const body = rewriteLinks(md.render(source), lang.slug);
      const html = renderPage({ lang, key, title, body, pages, isIndex });
      const file = path.join(OUT, lang.slug, key, "index.html");
      mkdirSync(path.dirname(file), { recursive: true });
      writeFileSync(file, html);
      written.push({ file, slug: lang.slug, key, html });
    };
    emit("", readme, true);
    keys.forEach((k) => emit(k, sources[k], false));
    cpSync(path.join(dir, "imgs"), path.join(OUT, lang.slug, "imgs"), { recursive: true });
  }
  checkLinks(written);
  console.log(`文档站已生成：${written.length} 个页面 → ${path.relative(ROOT, OUT)}`);
}

// 构建期校验：站内页面、锚点、图片必须都存在
function checkLinks(written) {
  const ids = new Map(written.map((w) => [`/docs/${w.slug}/${w.key ? `${w.key}/` : ""}`, new Set([...w.html.matchAll(/ id="([^"]+)"/g)].map((m) => m[1]))]));
  const errors = [];
  for (const w of written) {
    const self = `/docs/${w.slug}/${w.key ? `${w.key}/` : ""}`;
    for (const [, href] of w.html.matchAll(/href="([^"]+)"/g)) {
      if (!/^(#|\/docs\/(zh|tw|en|ja|es|pt)\/)/.test(href)) continue;
      const [pagePart, hash] = href.split("#");
      const target = pagePart || self;
      if (!ids.has(target)) {
        errors.push(`${self}: 页面不存在 ${href}`);
      } else if (hash && !ids.get(target).has(decodeURIComponent(hash))) {
        errors.push(`${self}: 锚点不存在 ${href}`);
      }
    }
    for (const [, src] of w.html.matchAll(/src="(\/docs\/[^"]+\.png)"/g)) {
      if (!existsSync(path.join(ROOT, "web", src))) errors.push(`${self}: 图片不存在 ${src}`);
    }
  }
  if (errors.length) {
    console.error(errors.join("\n"));
    process.exit(1);
  }
}

build();
