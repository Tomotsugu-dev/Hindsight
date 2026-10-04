import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { ChevronDown, ChevronRight, ChevronUp, EyeOff, Info } from "lucide-react";
import { useDurationFormatter } from "../../utils/duration";
import { useIsDark } from "../../hooks/useTheme";
import { adjustCategoryColor } from "../../utils/categoryColor";
import { ignoreKeywordFromTitle } from "../../utils/ignoreKeyword";
import { stripAppSuffix } from "../../utils/windowTitle";
import { logError } from "../../lib/logger";
import { api, type AppGroup, type TitleUsage } from "../../api/hindsight";
import { useSettings } from "../../state/settings";
import { groupBySite, type SiteGroup } from "./groupBySite";
import styles from "./AppDetailDrawer.module.css";

/** 「未识别网站」组的展开状态 key（域名里不可能出现 NUL，不会撞） */
const UNKNOWN_SITE_KEY = "\u0000unknown";

function siteKey(g: SiteGroup): string {
  return g.host ?? UNKNOWN_SITE_KEY;
}

interface WindowTitlesProps {
  app: {
    /** 显示名 —— 标题结尾的 " - {app名}" 按它剥掉 */
    name: string;
    groupId: string;
    /** 组里一个真实的 process_name；组列表拉不到时忽略规则只建在它上面 */
    iconProcess: string;
    color: string;
  };
  /** 后端按 (标题, 域名) 聚合好的用时 */
  titles: TitleUsage[];
  /** 代表进程是否浏览器 —— 是却一个域名都没有时，列表上方解释一句 */
  isBrowser: boolean;
  /** 只显示前几行，其余折叠；不传全部显示 */
  limit?: number;
  /** 每行的时长进度条；放在窄卡片里时关掉，留给标题 */
  showBars?: boolean;
  /** 外层容器的 class；列表为空时整个不渲染 */
  className?: string;
}

/**
 * 「具体在干啥」：按窗口标题列出用时，有网站域名时按网站分组，每行可以忽略这个窗口。
 * 应用详情抽屉和全部历史的窗口详情共用。换应用、换范围时由调用方换 key 重置。
 */
export function WindowTitles({
  app,
  titles,
  isBrowser,
  limit,
  showBars = true,
  className,
}: WindowTitlesProps) {
  const { t } = useTranslation();
  const fmtHM = useDurationFormatter();
  const isDark = useIsDark();
  const { settings } = useSettings();

  // —— 忽略窗口（写进 settings 的 ignore 规则，行照常记录仅不计入统计）——
  // 就地反馈：成功后本地隐藏对应行 + 一条可撤销的通知；真正的过滤在后端
  // 查询里（excluded=0），下次拉取自然生效。回看/删除的全集在 分类→应用 页。
  const [ignoredKeys, setIgnoredKeys] = useState<Set<string>>(new Set());
  const [ignoreNotice, setIgnoreNotice] = useState<{
    keyword: string;
    count: number;
    targets: string[];
  } | null>(null);
  const [ignoreBusy, setIgnoreBusy] = useState(false);
  // 「按网站」分组的展开集合：null = 默认态（只展开时长最多的第一组），
  // 用户点过之后才具体化。
  const [openKeys, setOpenKeys] = useState<Set<string> | null>(null);
  // 传了 limit 时：顶层列表是否展开全部、哪些网站组展开了全部页面
  const [showAll, setShowAll] = useState(false);
  const [allPagesKeys, setAllPagesKeys] = useState<Set<string>>(new Set());
  // 跨 OS 合并组：规则要覆盖组内每个 process_name（mac "Code" + win
  // "Visual Studio Code"），组列表拉一次就够。
  const groupsRef = useRef<AppGroup[] | null>(null);
  const noticeTimer = useRef<number | null>(null);

  useEffect(
    () => () => {
      if (noticeTimer.current !== null) window.clearTimeout(noticeTimer.current);
    },
    [],
  );

  const resolveRuleTargets = useCallback(
    async (groupId: string, iconProcess: string) => {
      try {
        if (!groupsRef.current) groupsRef.current = await api.listAppGroups();
        const g = groupsRef.current.find((grp) => grp.id === groupId);
        const names = g?.members.map((m) => m.processName) ?? [];
        return names.length > 0 ? names : [iconProcess];
      } catch {
        // 组列表拉不到就退化成只对代表进程建规则——宁可少盖也别整个失败
        return [iconProcess];
      }
    },
    [],
  );

  const ignoreTitle = useCallback(
    async (rawTitle: string) => {
      if (ignoreBusy) return;
      const keyword = ignoreKeywordFromTitle(rawTitle);
      if (!keyword) return;
      setIgnoreBusy(true);
      try {
        const targets = await resolveRuleTargets(app.groupId, app.iconProcess);
        let count = 0;
        for (const p of targets) {
          count += (await api.addIgnoreRule(p, keyword)).reappliedRows;
        }
        setIgnoredKeys((prev) => new Set(prev).add(keyword));
        setIgnoreNotice({ keyword, count, targets });
        if (noticeTimer.current !== null) {
          window.clearTimeout(noticeTimer.current);
        }
        noticeTimer.current = window.setTimeout(() => setIgnoreNotice(null), 8000);
      } catch (e) {
        logError("appDetail.ignore", e);
      } finally {
        setIgnoreBusy(false);
      }
    },
    [app.groupId, app.iconProcess, ignoreBusy, resolveRuleTargets],
  );

  const undoIgnore = useCallback(async () => {
    const n = ignoreNotice;
    if (!n || ignoreBusy) return;
    setIgnoreBusy(true);
    try {
      for (const p of n.targets) {
        await api.removeIgnoreRule(p, n.keyword);
      }
      setIgnoredKeys((prev) => {
        const next = new Set(prev);
        next.delete(n.keyword);
        return next;
      });
      setIgnoreNotice(null);
    } catch (e) {
      logError("appDetail.ignoreUndo", e);
    } finally {
      setIgnoreBusy(false);
    }
  }, [ignoreNotice, ignoreBusy]);

  // 页面行原料：后端已按 (标题, 域名) 聚合，这里剥 app 名后缀、隐藏刚被忽略的行
  // （撤销即恢复；下次真实拉取由后端 excluded=0 过滤）。纯标题列表与「按网站」
  // 分组共用这一份，两种视图对"哪些行可见"永远一致。
  const pageRows = useMemo(
    () =>
      titles
        .map((tu) => ({
          title: stripAppSuffix(tu.title, app.name),
          secs: tu.secs,
          host: tu.host,
        }))
        .filter((row) => !ignoredKeys.has(ignoreKeywordFromTitle(row.title))),
    [titles, app.name, ignoredKeys],
  );

  // "具体在干啥"：跨域名合并同标题、降序（= 抹掉 host 后的单组分组）
  const byTitle = useMemo(
    () => groupBySite(pageRows.map((r) => ({ ...r, host: null })))[0]?.pages ?? [],
    [pageRows],
  );
  const titleMax = useMemo(() => Math.max(...byTitle.map((x) => x.secs), 1), [byTitle]);

  // 「按网站」：**数据里有域名才分组**——记录开关只管之后的新行，关掉它不该让
  // 已记录的域名消失。分组保留 host 维度（同标题不同网站分开计）。
  const hasHost = pageRows.some((r) => r.host !== null);
  const siteGroups = useMemo<SiteGroup[]>(
    () => (hasHost ? groupBySite(pageRows) : []),
    [hasHost, pageRows],
  );
  const siteMax = useMemo(() => Math.max(...siteGroups.map((g) => g.secs), 1), [siteGroups]);
  // 组内页面已按秒数降序，每组取首项即可
  const pageMax = useMemo(
    () => Math.max(...siteGroups.map((g) => g.pages[0]?.secs ?? 0), 1),
    [siteGroups],
  );
  // 浏览器应用却一个域名都没有（升级前的记录 / 未授权 / 不支持的浏览器或系统）：
  // 在标题列表上方解释一句。用户自己关了记录开关则不提示。
  const showNoHostHint =
    isBrowser && !hasHost && pageRows.length > 0 && settings?.recordBrowserHost !== false;
  const openSet = useMemo<Set<string>>(() => {
    if (openKeys) return openKeys;
    return siteGroups.length > 0 ? new Set([siteKey(siteGroups[0])]) : new Set();
  }, [openKeys, siteGroups]);
  const toggleSite = (key: string) => {
    setOpenKeys((prev) => {
      const next = new Set(prev ?? openSet);
      if (next.has(key)) next.delete(key);
      else next.add(key);
      return next;
    });
  };
  const toggleAllPages = (key: string) => {
    setAllPagesKeys((prev) => {
      const next = new Set(prev);
      if (next.has(key)) next.delete(key);
      else next.add(key);
      return next;
    });
  };

  // 忽略掉最后一行后列表会空，但通知条（含撤销）必须还在
  if (byTitle.length === 0 && !ignoreNotice) return null;

  const fmtSecs = (secs: number): string => fmtHM(Math.max(1, Math.round(secs / 60)));
  const barColor = `color-mix(in oklab, ${adjustCategoryColor(app.color, isDark)} 70%, transparent)`;
  const cut = <T,>(rows: T[], all: boolean) =>
    limit === undefined || all ? rows : rows.slice(0, limit);
  const bar = (secs: number, max: number) =>
    showBars && (
      <span className={styles.titleBarWrap}>
        <span
          className={styles.titleBar}
          style={{ width: `${(secs / max) * 100}%`, background: barColor }}
        />
      </span>
    );

  // 超过 limit 时的「展开剩余 N 项 / 收起」
  const moreToggle = (total: number, expanded: boolean, onToggle: () => void) =>
    limit !== undefined &&
    total > limit && (
      <button type="button" className={styles.moreBtn} onClick={onToggle}>
        {expanded ? (
          <>
            <ChevronUp size={14} strokeWidth={2} aria-hidden />
            {t("components.rankedList.collapse")}
          </>
        ) : (
          <>
            <ChevronDown size={14} strokeWidth={2} aria-hidden />
            {t("components.rankedList.expand", { count: total - limit })}
          </>
        )}
      </button>
    );

  // 页面行：纯标题列表与「按网站」分组共用同一套行（含忽略按钮）
  const renderPageRow = (row: { title: string; secs: number }, max: number, key: string) => (
    <li key={key} className={styles.titleRow}>
      <span className={styles.titleName} title={row.title || t("appDetail.untitled")}>
        {row.title || t("appDetail.untitled")}
      </span>
      {bar(row.secs, max)}
      <span className={styles.titleTime}>{fmtSecs(row.secs)}</span>
      {row.title !== "" && (
        <button
          type="button"
          className={styles.ignoreBtn}
          disabled={ignoreBusy}
          aria-label={t("appDetail.ignore.button")}
          title={t("appDetail.ignore.button")}
          onClick={() => void ignoreTitle(row.title)}
        >
          <EyeOff size={14} strokeWidth={2} />
        </button>
      )}
    </li>
  );

  return (
    <section className={className}>
      {ignoreNotice && (
        <div className={styles.ignoreNotice} role="status">
          <span className={styles.ignoreNoticeText}>
            {t("appDetail.ignore.done", { count: ignoreNotice.count })}
          </span>
          <button
            type="button"
            className={styles.ignoreUndoBtn}
            onClick={() => void undoIgnore()}
            disabled={ignoreBusy}
          >
            {t("appDetail.ignore.undo")}
          </button>
        </div>
      )}
      {hasHost ? (
        <>
          <h3 className={styles.sectionTitle}>{t("appDetail.sites.title")}</h3>
          <ul className={styles.siteList}>
            {cut(siteGroups, showAll).map((g) => {
              const key = siteKey(g);
              const isOpen = openSet.has(key);
              const isUnknown = g.host === null;
              const label = g.host ?? t("appDetail.sites.unknown");
              const allPages = allPagesKeys.has(key);
              return (
                <li key={key} className={styles.siteGroup}>
                  <button
                    type="button"
                    className={styles.siteHead}
                    onClick={() => toggleSite(key)}
                    aria-expanded={isOpen}
                    title={isUnknown ? t("appDetail.sites.unknownHint") : label}
                  >
                    <ChevronRight
                      size={14}
                      strokeWidth={2}
                      className={`${styles.siteChevron} ${isOpen ? styles.siteChevronOpen : ""}`}
                      aria-hidden
                    />
                    <span
                      className={`${styles.siteHost} ${isUnknown ? styles.siteHostUnknown : ""}`}
                    >
                      {label}
                    </span>
                    {isUnknown && (
                      <Info size={13} strokeWidth={2} className={styles.siteInfo} aria-hidden />
                    )}
                    {bar(g.secs, siteMax)}
                    <span className={styles.titleTime}>{fmtSecs(g.secs)}</span>
                  </button>
                  {isOpen && (
                    <>
                      <ul className={styles.pageList}>
                        {cut(g.pages, allPages).map((p, i) =>
                          renderPageRow(p, pageMax, `${key}:${i}`),
                        )}
                      </ul>
                      {moreToggle(g.pages.length, allPages, () => toggleAllPages(key))}
                    </>
                  )}
                </li>
              );
            })}
          </ul>
          {moreToggle(siteGroups.length, showAll, () => setShowAll((v) => !v))}
        </>
      ) : (
        <>
          {showNoHostHint && (
            <p className={styles.siteHint}>
              <Info size={14} strokeWidth={2} aria-hidden />
              <span>{t("appDetail.sites.noHost")}</span>
            </p>
          )}
          <ul className={styles.titleList}>
            {cut(byTitle, showAll).map((row, i) => renderPageRow(row, titleMax, String(i)))}
          </ul>
          {moreToggle(byTitle.length, showAll, () => setShowAll((v) => !v))}
        </>
      )}
    </section>
  );
}
