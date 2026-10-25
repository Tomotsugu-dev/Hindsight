// 只处理 /themes/data/* 和 /assets/video/*（web/wrangler.jsonc 的 run_worker_first），其它请求由静态
// 资源直接响应。/themes/data/* 转发主题文件，见 themes-data.js。
//
// 静态资源不支持 Range 请求：带 Range 也返回 200 和整个文件。Safari 播放 <video> 要求服务器
// 支持按字节范围取（先发 Range: bytes=0-1 探测，拿不到 206 就不播），所以视频在这里切片，
// 返回 206。只支持单个范围；多个范围按 RFC 9110 退回 200 整个文件。

import { serveThemeData, THEMES_DATA_PREFIX } from "./themes-data.js";

export default {
  async fetch(request, env, ctx) {
    if (new URL(request.url).pathname.startsWith(THEMES_DATA_PREFIX)) {
      return serveThemeData(request);
    }

    const res = await env.ASSETS.fetch(request);
    if (res.status !== 200) return res;

    const headers = new Headers(res.headers);
    headers.set("Accept-Ranges", "bytes");
    const rangeHeader = request.headers.get("Range");
    const ifRange = request.headers.get("If-Range");
    // 没带 Range、HEAD、或文件已变（If-Range 不匹配）：整个返回
    if (!rangeHeader || request.method === "HEAD" || (ifRange && ifRange !== res.headers.get("ETag"))) {
      return new Response(res.body, { status: 200, headers });
    }

    let size = Number(res.headers.get("Content-Length"));
    let body = res.body;
    if (!(size > 0)) {
      // 有的环境（本地 wrangler dev）不给 Content-Length，只能读完整个文件才知道大小
      const buf = await res.arrayBuffer();
      size = buf.byteLength;
      body = new Blob([buf]).stream();
    }

    const range = parseRange(rangeHeader, size);
    if (!range) return new Response(body, { status: 200, headers });
    if (range === "unsatisfiable") {
      body.cancel();
      headers.set("Content-Range", `bytes */${size}`);
      headers.delete("Content-Length");
      return new Response(null, { status: 416, headers });
    }

    const [start, end] = range;
    const { readable, writable } = new FixedLengthStream(end - start + 1);
    ctx.waitUntil(pipeRange(body, writable, start, end));
    headers.set("Content-Range", `bytes ${start}-${end}/${size}`);
    headers.set("Content-Length", String(end - start + 1));
    return new Response(readable, { status: 206, headers });
  },
};

// "bytes=0-1"、"bytes=100-"、"bytes=-500" → [start, end]（含 end）；
// 不是单个字节范围 → null；起点超出文件 → "unsatisfiable"
function parseRange(header, size) {
  const m = /^bytes=(\d*)-(\d*)$/.exec(header.trim());
  if (!m || (m[1] === "" && m[2] === "")) return null;
  if (m[1] === "") {
    const suffix = Number(m[2]);
    return suffix === 0 ? "unsatisfiable" : [Math.max(size - suffix, 0), size - 1];
  }
  const start = Number(m[1]);
  const end = m[2] === "" ? size - 1 : Math.min(Number(m[2]), size - 1);
  if (start >= size) return "unsatisfiable";
  return start > end ? null : [start, end];
}

// 从头读整个文件，只把 [start, end] 之间的字节写出去，写够就停
async function pipeRange(body, writable, start, end) {
  const reader = body.getReader();
  const writer = writable.getWriter();
  let pos = 0;
  try {
    while (pos <= end) {
      const { done, value } = await reader.read();
      if (done) break;
      const from = Math.max(start - pos, 0);
      const to = Math.min(end + 1 - pos, value.byteLength);
      pos += value.byteLength;
      if (from < to) await writer.write(value.subarray(from, to));
    }
    await writer.close();
  } catch (err) {
    // 多半是浏览器提前断开（拖动进度条时常见）
    await writer.abort(err).catch(() => {});
  } finally {
    reader.cancel().catch(() => {});
  }
}
