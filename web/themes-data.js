// /themes/data/<文件名> → hindsight-plugins 的 stats 分支里 themes/ 下的同名文件。
//
// 浏览器不直接读 raw.githubusercontent.com：中国大陆常常连不上它，官网能打开。文件由机器人生成，
// 格式见 docs/design/themes/zh/market.md。只转发 index.json 和 <id>.json，别的路径一律 404，
// 不当成能转发任何地址的代理。

export const THEMES_DATA_PREFIX = "/themes/data/";

const SOURCE = "https://raw.githubusercontent.com/Tomotsugu-dev/hindsight-plugins/stats/themes/";

// 主题 id 加 .json，id 的规则和 Hindsight 一样。index.json 也合这个规则
const FILE_NAME = /^[a-z0-9](?:[a-z0-9-]{0,61}[a-z0-9])?\.json$/;

// 机器人推送以后，raw 自己最多缓存 5 分钟；这里再缓存 1 分钟，合并 PR 后 10 分钟内能看到
const CACHE_SECONDS = 60;

/** 返回路径对应的 raw 地址；不是要转发的文件时返回 null。 */
export function themeDataSource(pathname) {
  if (!pathname.startsWith(THEMES_DATA_PREFIX)) return null;
  const name = pathname.slice(THEMES_DATA_PREFIX.length);
  return FILE_NAME.test(name) ? SOURCE + name : null;
}

/** 转发一个主题文件。fetchSource 是测试时换掉 fetch 用的。 */
export async function serveThemeData(request, fetchSource = fetch) {
  if (request.method !== "GET" && request.method !== "HEAD") {
    return new Response(null, { status: 405, headers: { Allow: "GET, HEAD" } });
  }
  const source = themeDataSource(new URL(request.url).pathname);
  if (source === null) return new Response("Not found", { status: 404 });

  const upstream = await fetchSource(source, {
    cf: { cacheTtl: CACHE_SECONDS, cacheEverything: true },
  });
  // 主题下架以后，它的 <id>.json 被机器人删掉，raw 返回 404
  if (upstream.status === 404) return new Response("Not found", { status: 404 });
  if (!upstream.ok) return new Response("Bad gateway", { status: 502 });
  return new Response(request.method === "HEAD" ? null : upstream.body, {
    headers: {
      "Content-Type": "application/json; charset=utf-8",
      "Cache-Control": `public, max-age=${CACHE_SECONDS}`,
    },
  });
}
