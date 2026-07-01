import { useEffect, useMemo, useRef } from "react";
import { createPortal } from "react-dom";
import { useTranslation } from "react-i18next";
import { X } from "lucide-react";
import { AppIcon } from "../AppIcon/AppIcon";
import { EmptyHint } from "../EmptyHint/EmptyHint";
import { useFocusTrap } from "../../hooks/useFocusTrap";
import { useAppDetail, type DetailScope } from "../../hooks/useAppDetail";
import { useDurationFormatter } from "../../utils/duration";
import { useIsDark } from "../../hooks/useTheme";
import { adjustCategoryColor } from "../../utils/categoryColor";
import type { DetailBucket } from "../../api/hindsight";
import styles from "./AppDetailDrawer.module.css";

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
  const panelRef = useRef<HTMLDivElement>(null);

  const { detail, loading } = useAppDetail(
    scope,
    offset,
    app?.iconProcess ?? null,
    deviceId,
  );

  useFocusTrap(app !== null, panelRef);

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

  // "具体在干啥"：后端已按原始 window_title 聚合，这里剥 app 名后缀再合并、降序
  const byTitle = useMemo(() => {
    const appName = app?.name ?? "";
    const m = new Map<string, number>();
    for (const tu of detail?.titles ?? []) {
      const key = stripAppSuffix(tu.title, appName);
      m.set(key, (m.get(key) ?? 0) + tu.secs);
    }
    return [...m.entries()]
      .map(([title, secs]) => ({ title, secs }))
      .sort((a, b) => b.secs - a.secs);
  }, [detail?.titles, app?.name]);
  const titleMax = useMemo(
    () => Math.max(...byTitle.map((x) => x.secs), 1),
    [byTitle],
  );

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

  return createPortal(
    <div className={styles.backdrop} onMouseDown={onClose} role="presentation">
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

              {/* 具体在干啥：按窗口标题 */}
              {byTitle.length > 0 && (
                <section className={styles.section}>
                  <ul className={styles.titleList}>
                    {byTitle.map((row, i) => (
                      <li key={i} className={styles.titleRow}>
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
                              width: `${(row.secs / titleMax) * 100}%`,
                              background: `color-mix(in oklab, ${adjustCategoryColor(app.color, isDark)} 70%, transparent)`,
                            }}
                          />
                        </span>
                        <span className={styles.titleTime}>
                          {fmtSecs(row.secs)}
                        </span>
                      </li>
                    ))}
                  </ul>
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
