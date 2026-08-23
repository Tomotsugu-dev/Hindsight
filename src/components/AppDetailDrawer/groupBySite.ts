/** 应用详情「按网站」分组：把 (标题, 域名, 秒数) 行折成 域名 → 页面 两级。 */

export interface PageRow {
  title: string;
  secs: number;
}

export interface SiteGroup {
  /** 域名；null = 「未识别网站」（升级前的记录 / 未授权 / 不支持的浏览器） */
  host: string | null;
  /** 组内页面秒数之和 */
  secs: number;
  /** 组内页面，按秒数降序 */
  pages: PageRow[];
}

export interface SiteInputRow {
  title: string;
  secs: number;
  host: string | null;
}

/**
 * 分组规则：
 * - 同域名下同标题合并（调用方已剥掉 app 名后缀）；
 * - 组按总秒数降序，页面在组内按秒数降序；
 * - 没有域名的行归入 host=null 的「未识别网站」组，**固定排最后**——它是
 *   "还有多少时间没能按网站归类"的诚实展示，不该和真实网站争位次。
 */
export function groupBySite(rows: SiteInputRow[]): SiteGroup[] {
  const byHost = new Map<string | null, Map<string, number>>();
  for (const r of rows) {
    const host = r.host && r.host.trim() !== "" ? r.host : null;
    let pages = byHost.get(host);
    if (!pages) {
      pages = new Map();
      byHost.set(host, pages);
    }
    pages.set(r.title, (pages.get(r.title) ?? 0) + r.secs);
  }
  const groups: SiteGroup[] = [];
  let unknown: SiteGroup | null = null;
  for (const [host, pages] of byHost) {
    const list = [...pages.entries()]
      .map(([title, secs]) => ({ title, secs }))
      .sort((a, b) => b.secs - a.secs);
    const g: SiteGroup = {
      host,
      secs: list.reduce((s, p) => s + p.secs, 0),
      pages: list,
    };
    if (host === null) unknown = g;
    else groups.push(g);
  }
  groups.sort((a, b) => b.secs - a.secs);
  if (unknown) groups.push(unknown);
  return groups;
}
