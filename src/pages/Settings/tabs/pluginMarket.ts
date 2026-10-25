// 插件市场卡片和搜索框用到的小函数。
import type { OnlinePlugin } from "../../../api/plugins";
import { pickDescription } from "../../../utils/pickDescription";

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

/** 插件仓库的 GitHub 地址。格式不对就是 null，卡片上不显示链接。 */
export function repoUrl(repo: string): string | null {
  return REPO.test(repo) ? `https://github.com/${repo}` : null;
}
