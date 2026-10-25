// 读主题列表 /themes/data/index.json。文件由 hindsight-plugins 的机器人生成，字段见
// docs/design/themes/zh/market.md。
import type { ColorMode, PageLayout } from "@app/lib/theme";

const INDEX_FORMAT_VERSION = 1;

/** index.json 里的一个主题。 */
export interface ThemeEntry {
  id: string;
  name: string;
  author: string;
  description: Record<string, string>;
  version: string;
  layout: PageLayout;
  mode: ColorMode;
  tokens: Record<string, string>;
  repo: string;
  homepage: string;
  release_url: string;
  downloads: number;
}

/**
 * 读主题列表，保持文件里的顺序（已经按下载次数排好）。
 *
 * 读不到、不是 JSON、格式版本不认识时抛错，消息就是给用户看的原因。某一条字段不对只跳过
 * 那一条，其余照常显示。
 */
export async function loadIndex(fetchImpl: typeof fetch = fetch): Promise<ThemeEntry[]> {
  const res = await fetchImpl("/themes/data/index.json");
  if (!res.ok) throw new Error(`HTTP ${res.status}`);
  let data: unknown;
  try {
    data = await res.json();
  } catch {
    throw new Error("index.json is not JSON");
  }
  if (!isObject(data) || data.format_version !== INDEX_FORMAT_VERSION) {
    const version = isObject(data) ? String(data.format_version) : "?";
    throw new Error(`index.json format ${version} is not supported`);
  }
  if (!Array.isArray(data.themes)) throw new Error("index.json has no themes");
  return data.themes.filter(isThemeEntry);
}

function isObject(v: unknown): v is Record<string, unknown> {
  return typeof v === "object" && v !== null && !Array.isArray(v);
}

function isStringMap(v: unknown): v is Record<string, string> {
  return isObject(v) && Object.values(v).every((value) => typeof value === "string");
}

/** 链接只收 https：别的协议（javascript: 之类）点了会在官网上运行。 */
function isHttps(v: unknown): v is string {
  return typeof v === "string" && v.startsWith("https://");
}

function isThemeEntry(v: unknown): v is ThemeEntry {
  return (
    isObject(v) &&
    ["id", "name", "author", "version", "repo"].every((key) => typeof v[key] === "string") &&
    isStringMap(v.description) &&
    (v.layout === "card" || v.layout === "flat") &&
    (v.mode === "light" || v.mode === "dark") &&
    isStringMap(v.tokens) &&
    isHttps(v.homepage) &&
    isHttps(v.release_url) &&
    typeof v.downloads === "number"
  );
}
