export const BASE_PATH = "/internal/github-insights";

const SECURITY_HEADERS = {
  "Cache-Control": "private, no-store",
  "Content-Security-Policy": "default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data:; connect-src 'self'; base-uri 'none'; form-action 'none'; frame-ancestors 'none'; object-src 'none'",
  "X-Content-Type-Options": "nosniff",
  "X-Frame-Options": "DENY",
  "Referrer-Policy": "same-origin",
  "X-Robots-Tag": "noindex, nofollow, noarchive",
  "Vary": "Cookie",
};

function response(body, status = 200, type = "application/json; charset=utf-8", extra = {}) {
  return new Response(type.startsWith("application/json") ? JSON.stringify(body) : body, {
    status,
    headers: { ...SECURITY_HEADERS, "Content-Type": type, ...extra },
  });
}

// No Static Assets binding: all dashboard resources pass through the same
// Access check, including when requested via a preview or workers.dev hostname.
// ctx.access is supplied and verified by Cloudflare, never by request headers.
export function createHandler({ html, css, javascript, downloadsModule = "", repositories }) {
  const allowedRepositories = new Set(repositories);
  return {
    async fetch(request, env, ctx) {
      const result = await handleRequest(request, env, ctx);
      if (request.method !== "HEAD") return result;
      // HEAD has the same status and headers as GET, including error replies,
      // but must not send JSON "null" or any other response body.
      return new Response(null, {
        status: result.status,
        statusText: result.statusText,
        headers: result.headers,
      });
    },
  };

  async function handleRequest(request, env, ctx) {
      const url = new URL(request.url);
      if (url.hostname !== (env.INSIGHTS_HOST || "hindsight.kyosweb.com")) {
        return response({ error: "Not found" }, 404);
      }
      const audiences = Array.isArray(ctx?.access?.aud) ? ctx.access.aud : [ctx?.access?.aud];
      if (!env.ACCESS_AUD || !ctx?.access || !audiences.includes(env.ACCESS_AUD)) {
        return response({ error: "Internal sign-in required" }, 403);
      }
      if (request.method !== "GET" && request.method !== "HEAD") {
        return response({ error: "Method not allowed" }, 405, undefined, { Allow: "GET, HEAD" });
      }
      if (url.pathname === BASE_PATH) {
        return response(null, 308, "text/plain; charset=utf-8", { Location: `${BASE_PATH}/` });
      }
      const resources = new Map([
        [`${BASE_PATH}/`, [html, "text/html; charset=utf-8"]],
        [`${BASE_PATH}/dashboard.css`, [css, "text/css; charset=utf-8"]],
        [`${BASE_PATH}/dashboard.js`, [javascript, "application/javascript; charset=utf-8"]],
        [`${BASE_PATH}/downloads.mjs`, [downloadsModule, "application/javascript; charset=utf-8"]],
      ]);
      if (resources.has(url.pathname)) {
        const [body, type] = resources.get(url.pathname);
        return response(request.method === "HEAD" ? null : body, 200, type);
      }
      try {
        if (!env.INSIGHTS) return response({ error: "Insights storage is unavailable" }, 503);
        if (url.pathname === `${BASE_PATH}/api/status`) {
          const status = await env.INSIGHTS.get("status:v1", "json");
          const result = status || {
            version: 1,
            lastAttemptAt: null,
            lastSuccessAt: null,
            repositories: repositories.map((repository) => ({ repository, firstCollectedAt: null, lastSuccessAt: null })),
            errors: [],
          };
          return response(request.method === "HEAD" ? null : result);
        }
        if (url.pathname === `${BASE_PATH}/api/repository`) {
          const repository = url.searchParams.get("repo");
          if (!allowedRepositories.has(repository)) return response({ error: "Unknown repository" }, 404);
          const history = await env.INSIGHTS.get(`history:v1:${repository}`, "json");
          if (!history) return response({ error: "尚未完成首次采集，请等待定时任务或手动运行 Insights workflow。" }, 404);
          return response(request.method === "HEAD" ? null : history);
        }
        return response({ error: "Not found" }, 404);
      } catch {
        // Never expose storage exceptions, configuration, or account details.
        return response({ error: "读取统计失败，请稍后重试。" }, 503);
      }
  }
}
