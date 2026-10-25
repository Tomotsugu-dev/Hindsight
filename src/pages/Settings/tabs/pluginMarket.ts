// 插件市场卡片和搜索框用到的小函数。
import type { OnlinePlugin } from "../../../api/plugins";
import { pickDescription } from "../../../utils/pickDescription";

/** 市场的筛选：下载最多是全部插件；官方插件只留官方的。两种都保持后端排好的下载次数顺序 */
export type MarketFilter = "popular" | "official";

/** GitHub 的仓库写法 owner/name；仓库名不能只有点（"." ".." 会变成别的地址） */
const REPO = /^[A-Za-z0-9-]+\/(?!\.+$)[A-Za-z0-9._-]+$/;

/** 名称、作者、仓库、界面语言的简介里含有搜索的字就算匹配，不分大小写。搜索框空着时都匹配。 */
export function matchesQuery(plugin: OnlinePlugin, query: string, language: string): boolean {
  const q = query.trim().toLocaleLowerCase();
  if (!q) return true;
  return [plugin.name, plugin.author, plugin.repo, pickDescription(plugin.description, language)]
    .map((text) => text.toLocaleLowerCase())
    .some((text) => text.includes(q));
}

/** 按筛选和搜索框留下要显示的插件，顺序不变。 */
export function filterMarket(
  plugins: OnlinePlugin[],
  filter: MarketFilter,
  query: string,
  language: string,
): OnlinePlugin[] {
  return plugins
    .filter((p) => filter === "popular" || p.official)
    .filter((p) => matchesQuery(p, query, language));
}

/** 插件仓库的 GitHub 地址。格式不对就是 null，卡片上不显示链接。 */
export function repoUrl(repo: string): string | null {
  return REPO.test(repo) ? `https://github.com/${repo}` : null;
}
