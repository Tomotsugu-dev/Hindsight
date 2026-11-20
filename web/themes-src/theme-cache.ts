// 主题页 /themes/ 用来缓存完整主题文件（/themes/data/<id>.json，含 theme.css）。
// 鼠标悬停/聚焦主题卡片时提前预取，点击「预览」时直接命中内存缓存。
import { isCustomTheme, type CustomTheme } from "@app/lib/theme";

const TIMEOUT_MS = 10_000;

const loaded = new Map<string, CustomTheme>();
const inFlight = new Map<string, Promise<CustomTheme>>();

/** 已缓存在内存中的完整主题；未加载过则返回 null。 */
export function getCachedTheme(id: string): CustomTheme | null {
  return loaded.get(id) ?? null;
}

/** 读取并缓存完整主题文件（并发请求会自动去重；失败不缓存以便下次重试）。 */
export function loadFullTheme(id: string, fetchImpl: typeof fetch = fetch): Promise<CustomTheme> {
  const hit = loaded.get(id);
  if (hit) return Promise.resolve(hit);

  const pending = inFlight.get(id);
  if (pending) return pending;

  const path = `/themes/data/${id}.json`;
  const task = fetchImpl(path, { signal: AbortSignal.timeout(TIMEOUT_MS) })
    .then(async (res) => {
      if (!res.ok) throw new Error(`${path}: HTTP ${res.status}`);
      const data: unknown = await res.json();
      if (!isCustomTheme(data) || data.id !== id) {
        throw new Error(`${path} is not the theme ${id}`);
      }
      loaded.set(id, data);
      return data;
    })
    .finally(() => {
      inFlight.delete(id);
    });

  inFlight.set(id, task);
  return task;
}

/** 悬停/聚焦时静默预取完整主题文件。 */
export function prefetchTheme(id: string, fetchImpl: typeof fetch = fetch): void {
  void loadFullTheme(id, fetchImpl).catch(() => {});
}

/** 仅供测试清理缓存用。 */
export function clearThemeCache(): void {
  loaded.clear();
  inFlight.clear();
}
