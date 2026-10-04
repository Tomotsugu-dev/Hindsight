import { useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { X } from "lucide-react";
import { AppIcon } from "../../components/AppIcon/AppIcon";
import { EmptyHint } from "../../components/EmptyHint/EmptyHint";
import { ScrollBox } from "../../components/ScrollBox/ScrollBox";
import {
  AppDetailDrawer,
  type AppDetailTarget,
} from "../../components/AppDetailDrawer/AppDetailDrawer";
import { WindowTitles } from "../../components/AppDetailDrawer/WindowTitles";
import type { AppDetail } from "../../api/hindsight";
import { useDurationFormatter } from "../../utils/duration";
import { parseDayKey, type DayRange } from "./heatmapGrid";
import { useAppRangeDetail } from "./useHistoryData";
import styles from "./HistoryPage.module.css";

/** 窗口标题默认显示的行数；全部历史的标题动辄上千条 */
const TITLE_LIMIT = 20;

/** 被点开的应用：排行行上已有的信息 */
export interface HistoryApp {
  name: string;
  groupId: string;
  iconProcess: string;
  categoryLabel?: string;
  color: string;
}

interface AppDetailCardProps {
  app: HistoryApp;
  /** 这个应用从第一条记录到今天的明细，页面拉好传进来（热力图也用它） */
  all: AppDetail | null;
  /** 全部历史的范围：第一条记录到今天 */
  allRange: DayRange;
  /** 现在看的范围：选中的那天，或者热力图上拖出来的那段，或者这一年、这个月 */
  scope: DayRange;
  selectedDay: string | null;
  deviceId?: string;
  onClose: () => void;
}

/** 见 MonthPage 同名函数：日期 → 相对今天的 dayOffset。 */
function dayOffsetForDate(date: Date): number {
  const today = new Date();
  today.setHours(0, 0, 0, 0);
  return Math.round((date.getTime() - today.getTime()) / 86400000);
}

/** 窗口详情卡：头部照应用详情抽屉，下面是这段范围的窗口标题（跟抽屉同一个组件）。 */
export function AppDetailCard({
  app,
  all,
  allRange,
  scope,
  selectedDay,
  deviceId,
  onClose,
}: AppDetailCardProps) {
  const { t, i18n } = useTranslation();
  const fmtHM = useDurationFormatter();
  const [drawerOpen, setDrawerOpen] = useState(false);

  // 看的就是全部历史时，标题直接用页面拉好的那份，不再拉一次
  const isAll = scope.from === allRange.from && scope.to === allRange.to;
  const scoped = useAppRangeDetail(isAll ? null : app.groupId, scope.from, scope.to, deviceId);
  const detail = isAll ? all : scoped.data;
  const loading = isAll ? all === null : scoped.loading;

  const activeKeys = useMemo(
    () => (all?.buckets ?? []).filter((b) => b.secs > 0).map((b) => b.key),
    [all],
  );
  const totalSecs = (all?.buckets ?? [])
    .filter((b) => b.key >= scope.from && b.key <= scope.to)
    .reduce((sum, b) => sum + b.secs, 0);

  const dateFmt = useMemo(
    () =>
      new Intl.DateTimeFormat(i18n.language, {
        year: "numeric",
        month: "short",
        day: "numeric",
      }),
    [i18n.language],
  );
  const fmtDay = (key: string) => dateFmt.format(parseDayKey(key));

  const drawerTarget: AppDetailTarget | null =
    drawerOpen && selectedDay !== null
      ? { ...app, minutes: Math.round(totalSecs / 60) }
      : null;

  return (
    <section className={styles.card}>
      <header className={styles.detailHead}>
        <AppIcon processName={app.iconProcess} fallbackColor={app.color} size={34} />
        <div className={styles.detailHeadText}>
          <div className={styles.detailName} title={app.name}>
            {app.name}
          </div>
          <div className={styles.detailSub}>
            {app.categoryLabel && (
              <span className={styles.detailCat}>
                <span className={styles.catDot} style={{ background: app.color }} aria-hidden />
                {app.categoryLabel}
              </span>
            )}
            {activeKeys.length > 0 && (
              <span>
                {t("history.app.firstLast", {
                  first: fmtDay(activeKeys[0]),
                  last: fmtDay(activeKeys[activeKeys.length - 1]),
                })}
              </span>
            )}
          </div>
        </div>
        <div className={styles.detailTotal}>{fmtHM(Math.round(totalSecs / 60))}</div>
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

      {selectedDay !== null && (
        <div className={styles.detailActions}>
          <button type="button" className={styles.textBtn} onClick={() => setDrawerOpen(true)}>
            {t("history.app.dayDetail")}
          </button>
        </div>
      )}

      {loading ? (
        <div className={styles.loading}>{t("history.loading")}</div>
      ) : (detail?.titles ?? []).length === 0 ? (
        <EmptyHint />
      ) : (
        <ScrollBox maxHeight={420}>
          <WindowTitles
            // 换应用、换范围时展开和忽略的就地状态回到默认
            key={`${app.groupId}|${scope.from}|${scope.to}|${deviceId ?? ""}`}
            app={app}
            titles={detail?.titles ?? []}
            isBrowser={detail?.isBrowser ?? false}
            limit={TITLE_LIMIT}
            showBars={false}
            className={styles.titlesBody}
          />
        </ScrollBox>
      )}

      <AppDetailDrawer
        app={drawerTarget}
        scope="day"
        offset={selectedDay === null ? 0 : dayOffsetForDate(parseDayKey(selectedDay))}
        deviceId={deviceId}
        onClose={() => setDrawerOpen(false)}
      />
    </section>
  );
}
