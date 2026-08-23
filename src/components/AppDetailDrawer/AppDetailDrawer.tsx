import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { useTranslation } from "react-i18next";
import { ChevronRight, EyeOff, Info, X } from "lucide-react";
import { AppIcon } from "../AppIcon/AppIcon";
import { EmptyHint } from "../EmptyHint/EmptyHint";
import { useFocusTrap } from "../../hooks/useFocusTrap";
import { useAppDetail, type DetailScope } from "../../hooks/useAppDetail";
import { useDurationFormatter } from "../../utils/duration";
import { useIsDark } from "../../hooks/useTheme";
import { adjustCategoryColor } from "../../utils/categoryColor";
import { ignoreKeywordFromTitle } from "../../utils/ignoreKeyword";
import { logError } from "../../lib/logger";
import { api, type AppGroup, type DetailBucket } from "../../api/hindsight";
import { useSettings } from "../../state/settings";
import { groupBySite, type SiteGroup } from "./groupBySite";
import styles from "./AppDetailDrawer.module.css";

/** 「未识别网站」组的展开状态 key（域名里不可能出现 NUL，不会撞） */
const UNKNOWN_SITE_KEY = "\u0000unknown";

function siteKey(g: SiteGroup): string {
  return g.host ?? UNKNOWN_SITE_KEY;
}

/** 被点击的排行行传进来的最小信息（其余明细抽屉自己拉）。 */
export interface AppDetailTarget {
  /** 显示名 */
  name: string;
  /** 稳定代表 process_name —— 后端据此解析 app_group 拉明细 */
  iconProcess: string;
  /** 分类显示名（来自排行行 subtitle）；可无 */
  categoryLabel?: string;
  /** 分类色 */
  color: string;
  /** 该 app 在当前范围的总时长（分钟）—— 直接复用排行行已算好的值 */
  minutes: number;
}

interface AppDetailDrawerProps {
  /** null = 抽屉关闭（不请求） */
  app: AppDetailTarget | null;
  /** 时间范围：日 / 周 / 月 */
  scope: DetailScope;
  /** 对应 scope 的 offset（dayOffset / weekOffset / monthOffset） */
  offset: number;
  deviceId?: string;
  onClose: () => void;
}

/** 去掉标题结尾冗余的 " - {app名}"（VS Code 等把 app 名拼在标题最后，纯重复）。 */
function stripAppSuffix(title: string, appName: string): string {
  const t = title.trim();
  if (!appName) return t;
  for (const sep of [" - ", " — ", " – "]) {
    const suffix = sep + appName;
    if (t.endsWith(suffix)) return t.slice(0, -suffix.length).trim();
  }
  return t;
}

/** 天粒度 key "YYYY-MM-DD" 按本地零点解析成 Date。 */
function keyDate(key: string): Date {
  return new Date(`${key}T00:00:00`);
}

/** 月：从所有天桶里挑 ~5 个均匀位置显示日号，做稀疏轴标。 */
function monthTicks(buckets: DetailBucket[]): string[] {
  const n = buckets.length;
  if (n === 0) return [];
  const idxs = [
    ...new Set([
      0,
      Math.floor(n * 0.25),
      Math.floor(n * 0.5),
      Math.floor(n * 0.75),
      n - 1,
    ]),
  ];
  return idxs.map((i) => String(keyDate(buckets[i].key).getDate()));
}

export function AppDetailDrawer({
  app,
  scope,
  offset,
  deviceId,
  onClose,
}: AppDetailDrawerProps) {
  const { t, i18n } = useTranslation();
  const fmtHM = useDurationFormatter();
  const isDark = useIsDark();
  const { settings } = useSettings();
  const panelRef = useRef<HTMLDivElement>(null);

  const { detail, loading } = useAppDetail(
    scope,
    offset,
    app?.iconProcess ?? null,
    deviceId,
  );

  useFocusTrap(app !== null, panelRef);

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
  // 用户点过之后才具体化——这样换 app 时一个 null 就回到默认。
  const [openKeys, setOpenKeys] = useState<Set<string> | null>(null);
  // 跨 OS 合并组：规则要覆盖组内每个 process_name（mac "Code" + win
  // "Visual Studio Code"），组列表整个抽屉生命周期拉一次就够。
  const groupsRef = useRef<AppGroup[] | null>(null);
  const noticeTimer = useRef<number | null>(null);

  // 换 app 清掉上一个 app 的就地状态
  useEffect(() => {
    setIgnoredKeys(new Set());
    setIgnoreNotice(null);
  }, [app?.iconProcess]);
  // 展开状态还要跟着时间范围走：换天/换设备后组的集合都变了，回到默认态
  useEffect(() => {
    setOpenKeys(null);
  }, [app?.iconProcess, scope, offset, deviceId]);
  useEffect(
    () => () => {
      if (noticeTimer.current !== null) window.clearTimeout(noticeTimer.current);
    },
    [],
  );

  const resolveRuleTargets = useCallback(async (iconProcess: string) => {
    try {
      if (!groupsRef.current) groupsRef.current = await api.listAppGroups();
      const g = groupsRef.current.find(
        (grp) =>
          grp.id === iconProcess ||
          grp.members.some((m) => m.processName === iconProcess),
      );
      const names = g?.members.map((m) => m.processName) ?? [];
      return names.length > 0 ? names : [iconProcess];
    } catch {
      // 组列表拉不到就退化成只对代表进程建规则——宁可少盖也别整个失败
      return [iconProcess];
    }
  }, []);

  const ignoreTitle = useCallback(
    async (rawTitle: string) => {
      if (!app || ignoreBusy) return;
      const keyword = ignoreKeywordFromTitle(rawTitle);
      if (!keyword) return;
      setIgnoreBusy(true);
      try {
        const targets = await resolveRuleTargets(app.iconProcess);
        let count = 0;
        for (const p of targets) {
          count += (await api.addIgnoreRule(p, keyword)).reappliedRows;
        }
        setIgnoredKeys((prev) => new Set(prev).add(keyword));
        setIgnoreNotice({ keyword, count, targets });
        if (noticeTimer.current !== null) {
          window.clearTimeout(noticeTimer.current);
        }
        noticeTimer.current = window.setTimeout(
          () => setIgnoreNotice(null),
          8000,
        );
      } catch (e) {
        logError("appDetail.ignore", e);
      } finally {
        setIgnoreBusy(false);
      }
    },
    [app, ignoreBusy, resolveRuleTargets],
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

  // Esc 关抽屉
  useEffect(() => {
    if (!app) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, [app, onClose]);

  const buckets = useMemo(() => detail?.buckets ?? [], [detail]);
  const maxBucket = useMemo(
    () => Math.max(...buckets.map((b) => b.secs), 1),
    [buckets],
  );

  // 页面行原料：后端已按 (标题, 域名) 聚合，这里剥 app 名后缀、隐藏刚被忽略的行
  // （撤销即恢复；下次真实拉取由后端 excluded=0 过滤）。纯标题列表与「按网站」
  // 分组共用这一份，两种视图对"哪些行可见"永远一致。
  const pageRows = useMemo(() => {
    const appName = app?.name ?? "";
    return (detail?.titles ?? [])
      .map((tu) => ({
        title: stripAppSuffix(tu.title, appName),
        secs: tu.secs,
        host: tu.host,
      }))
      .filter((row) => !ignoredKeys.has(ignoreKeywordFromTitle(row.title)));
  }, [detail?.titles, app?.name, ignoredKeys]);

  // "具体在干啥"：跨域名合并同标题、降序（= 抹掉 host 后的单组分组）
  const byTitle = useMemo(
    () =>
      groupBySite(pageRows.map((r) => ({ ...r, host: null })))[0]?.pages ?? [],
    [pageRows],
  );
  const titleMax = useMemo(
    () => Math.max(...byTitle.map((x) => x.secs), 1),
    [byTitle],
  );

  // 「按网站」：**数据里有域名才分组**——记录开关只管之后的新行，关掉它不该让
  // 已记录的域名从抽屉里消失。分组保留 host 维度（同标题不同网站分开计）。
  const hasHost = pageRows.some((r) => r.host !== null);
  const siteMode = hasHost;
  const siteGroups = useMemo<SiteGroup[]>(
    () => (hasHost ? groupBySite(pageRows) : []),
    [hasHost, pageRows],
  );
  const siteMax = useMemo(
    () => Math.max(...siteGroups.map((g) => g.secs), 1),
    [siteGroups],
  );
  // 组内页面已按秒数降序，每组取首项即可
  const pageMax = useMemo(
    () => Math.max(...siteGroups.map((g) => g.pages[0]?.secs ?? 0), 1),
    [siteGroups],
  );
  // 浏览器应用却一个域名都没有（升级前的记录 / 未授权 / 不支持的浏览器或系统）：
  // 在标题列表上方解释一句。用户自己关了记录开关则不提示。
  const showNoHostHint =
    Boolean(detail?.isBrowser) &&
    !hasHost &&
    pageRows.length > 0 &&
    settings?.recordBrowserHost !== false;
  const openSet = useMemo<Set<string>>(() => {
    if (openKeys) return openKeys;
    return siteGroups.length > 0
      ? new Set([siteKey(siteGroups[0])])
      : new Set();
  }, [openKeys, siteGroups]);
  const toggleSite = (key: string) => {
    setOpenKeys((prev) => {
      const next = new Set(prev ?? openSet);
      if (next.has(key)) next.delete(key);
      else next.add(key);
      return next;
    });
  };

  // 日期格式器跟随界面语言（hoist 出 map，避免每根柱子新建）
  const dateFmt = useMemo(
    () =>
      new Intl.DateTimeFormat(i18n.language, {
        month: "numeric",
        day: "numeric",
        weekday: "short",
      }),
    [i18n.language],
  );
  const weekdayFmt = useMemo(
    () => new Intl.DateTimeFormat(i18n.language, { weekday: "narrow" }),
    [i18n.language],
  );

  const fmtSecs = (secs: number): string =>
    fmtHM(Math.max(1, Math.round(secs / 60)));

  // 柱子 hover 文案：日=几点，周/月=哪天
  const bucketTip = (b: DetailBucket): string => {
    if (scope === "day") {
      return `${b.key.padStart(2, "0")}:00 · ${fmtSecs(b.secs)}`;
    }
    return `${dateFmt.format(keyDate(b.key))} · ${fmtSecs(b.secs)}`;
  };

  if (!app) return null;

  const hasData = buckets.some((b) => b.secs > 0) || byTitle.length > 0;
  const barColor = `color-mix(in oklab, ${adjustCategoryColor(app.color, isDark)} 70%, transparent)`;

  // 页面行：纯标题列表与「按网站」分组共用同一套行（含忽略按钮）
  const renderPageRow = (
    row: { title: string; secs: number },
    max: number,
    key: string,
  ) => (
    <li key={key} className={styles.titleRow}>
      <span
        className={styles.titleName}
        title={row.title || t("appDetail.untitled")}
      >
        {row.title || t("appDetail.untitled")}
      </span>
      <span className={styles.titleBarWrap}>
        <span
          className={styles.titleBar}
          style={{
            width: `${(row.secs / max) * 100}%`,
            background: barColor,
          }}
        />
      </span>
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

  return createPortal(
    // data-keeps-bar-selection:抽屉是从"选中某天"派生出来的,在它里面操作
    // 不该反过来清掉背后的选中(否则关掉抽屉,榜单已跳回整周口径)
    <div
      className={styles.backdrop}
      onMouseDown={onClose}
      role="presentation"
      data-keeps-bar-selection
    >
      {/* eslint-disable-next-line jsx-a11y/no-noninteractive-element-interactions */}
      <aside
        ref={panelRef}
        className={styles.panel}
        role="dialog"
        aria-modal="true"
        aria-label={app.name}
        onMouseDown={(e) => e.stopPropagation()}
      >
        <header className={styles.head}>
          <AppIcon
            processName={app.iconProcess}
            fallbackColor={app.color}
            size={34}
          />
          <div className={styles.headText}>
            <div className={styles.appName} title={app.name}>
              {app.name}
            </div>
            {app.categoryLabel ? (
              <div className={styles.appCat}>
                <span
                  className={styles.catDot}
                  style={{ background: app.color }}
                  aria-hidden
                />
                {app.categoryLabel}
              </div>
            ) : null}
          </div>
          <div className={styles.headTotal}>{fmtHM(app.minutes)}</div>
          <button
            type="button"
            className={styles.closeBtn}
            onClick={onClose}
            aria-label={t("common.close")}
            title={t("common.close")}
          >
            <X size={18} strokeWidth={2} />
          </button>
        </header>

        <div className={styles.body}>
          {loading ? (
            <div className={styles.loading}>
              <span className={styles.spinner} aria-hidden />
              {t("appDetail.loading")}
            </div>
          ) : !hasData ? (
            <EmptyHint />
          ) : (
            <>
              {/* 时间柱：日=24 根小时，周/月=每天一根 */}
              <section className={styles.section}>
                <div className={styles.chart}>
                  {buckets.map((b, i) => (
                    <div key={i} className={styles.bar} title={bucketTip(b)}>
                      <div
                        className={styles.fill}
                        style={{
                          height: `${(b.secs / maxBucket) * 100}%`,
                          background: adjustCategoryColor(app.color, isDark),
                        }}
                      />
                    </div>
                  ))}
                </div>
                {scope === "day" ? (
                  <div className={styles.axis}>
                    <span>0</span>
                    <span>6</span>
                    <span>12</span>
                    <span>18</span>
                    <span>24</span>
                  </div>
                ) : scope === "week" ? (
                  <div className={styles.axisWeek}>
                    {buckets.map((b, i) => (
                      <span key={i}>{weekdayFmt.format(keyDate(b.key))}</span>
                    ))}
                  </div>
                ) : (
                  <div className={styles.axis}>
                    {monthTicks(buckets).map((d, i) => (
                      <span key={i}>{d}</span>
                    ))}
                  </div>
                )}
              </section>

              {/* 具体在干啥：按窗口标题。忽略掉最后一行后列表会空，
                  但通知条（含撤销）必须还在——条件里带上 ignoreNotice */}
              {(byTitle.length > 0 || ignoreNotice) && (
                <section className={styles.section}>
                  {ignoreNotice && (
                    <div className={styles.ignoreNotice} role="status">
                      <span className={styles.ignoreNoticeText}>
                        {t("appDetail.ignore.done", {
                          count: ignoreNotice.count,
                        })}
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
                  {siteMode ? (
                    <>
                      <h3 className={styles.sectionTitle}>
                        {t("appDetail.sites.title")}
                      </h3>
                      <ul className={styles.siteList}>
                        {siteGroups.map((g) => {
                          const key = siteKey(g);
                          const isOpen = openSet.has(key);
                          const isUnknown = g.host === null;
                          const label = g.host ?? t("appDetail.sites.unknown");
                          return (
                            <li key={key} className={styles.siteGroup}>
                              <button
                                type="button"
                                className={styles.siteHead}
                                onClick={() => toggleSite(key)}
                                aria-expanded={isOpen}
                                title={
                                  isUnknown
                                    ? t("appDetail.sites.unknownHint")
                                    : label
                                }
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
                                  <Info
                                    size={13}
                                    strokeWidth={2}
                                    className={styles.siteInfo}
                                    aria-hidden
                                  />
                                )}
                                <span className={styles.titleBarWrap}>
                                  <span
                                    className={styles.titleBar}
                                    style={{
                                      width: `${(g.secs / siteMax) * 100}%`,
                                      background: barColor,
                                    }}
                                  />
                                </span>
                                <span className={styles.titleTime}>
                                  {fmtSecs(g.secs)}
                                </span>
                              </button>
                              {isOpen && (
                                <ul className={styles.pageList}>
                                  {g.pages.map((p, i) =>
                                    renderPageRow(p, pageMax, `${key}:${i}`),
                                  )}
                                </ul>
                              )}
                            </li>
                          );
                        })}
                      </ul>
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
                        {byTitle.map((row, i) =>
                          renderPageRow(row, titleMax, String(i)),
                        )}
                      </ul>
                    </>
                  )}
                </section>
              )}
            </>
          )}
        </div>
      </aside>
    </div>,
    document.body,
  );
}
