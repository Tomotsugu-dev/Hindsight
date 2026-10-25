// /demo/?theme=<id>：官网主题页的预览。向 /themes/data/<id>.json 要主题，在画页面之前套用。
// 必须在画页面之前：图表的颜色是 JS 取了写进 inline style 的，画完再换主题不会跟着变。
import { applyCustomTheme, isCustomTheme, type CustomTheme } from "@app/lib/theme";

// 和 Hindsight 的主题 id 规则一样
const ID = /^[a-z0-9](?:[a-z0-9-]{0,61}[a-z0-9])?$/;

// 读不到时，demo 不能一直空白
const TIMEOUT_MS = 10_000;

/** ?theme= 里的主题 id。没有 ?theme=，或者值不合 id 的规则时，返回 null。 */
export function previewThemeId(search: string): string | null {
  const id = new URLSearchParams(search).get("theme");
  return id !== null && ID.test(id) ? id : null;
}

/** 读一个主题的预览文件。读不到，或者内容不是这个主题时，抛错。 */
export async function fetchPreviewTheme(
  id: string,
  fetchImpl: typeof fetch = fetch,
): Promise<CustomTheme> {
  const path = `/themes/data/${id}.json`;
  const res = await fetchImpl(path, { signal: AbortSignal.timeout(TIMEOUT_MS) });
  if (!res.ok) throw new Error(`${path}: HTTP ${res.status}`);
  const theme: unknown = await res.json();
  if (!isCustomTheme(theme) || theme.id !== id) throw new Error(`${path} is not the theme ${id}`);
  return theme;
}

/**
 * 有 ?theme= 时套用那个主题。不写 localStorage：只是预览，不改访客自己选的主题。
 *
 * 返回要不要提示「读不到这个主题」。
 */
export async function applyPreviewTheme(search: string): Promise<boolean> {
  if (!new URLSearchParams(search).has("theme")) return false;
  const id = previewThemeId(search);
  try {
    if (id === null) throw new Error("?theme= is not a theme id");
    applyCustomTheme(await fetchPreviewTheme(id));
    return false;
  } catch (e) {
    console.error("[demo] preview theme", e);
    return true;
  }
}
