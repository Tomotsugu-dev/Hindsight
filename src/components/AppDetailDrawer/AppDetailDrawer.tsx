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
import { WindowTitles } from "./WindowTitles";
import styles from "./AppDetailDrawer.module.css";

/** 被点击的排行行传进来的最小信息（其余明细抽屉自己拉）。 */
export interface AppDetailTarget {
  /** 显示名 */
  name: string;
  /** 分组 ID —— 后端按它拉明细 */
  groupId: string;
  /** 组里一个真实的 process_name，用来查图标 */
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
    app?.groupId ?? null,
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

  const titles = detail?.titles ?? [];
  const hasData = buckets.some((b) => b.secs > 0) || titles.length > 0;

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

              {/* 具体在干啥：按窗口标题。换 app、换时间范围时重置展开和忽略的就地状态 */}
              <WindowTitles
                key={`${app.groupId}|${scope}|${offset}|${deviceId ?? ""}`}
                app={app}
                titles={titles}
                isBrowser={detail?.isBrowser ?? false}
                className={styles.section}
              />
            </>
          )}
        </div>
      </aside>
    </div>,
    document.body,
  );
}
