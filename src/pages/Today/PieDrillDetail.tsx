import { useMemo, type CSSProperties } from "react";
import { useTranslation } from "react-i18next";
import { ArrowLeft } from "lucide-react";
import type { AppUsage, Category } from "../../api/hindsight";
import type { BreakdownSlice } from "../../hooks/useSuperCategoryBreakdown";
import { AppIcon } from "../../components/AppIcon/AppIcon";
import { useDurationFormatter } from "../../utils/duration";
import { withViewTransition } from "../../utils/viewTransition";
import { displayAppName } from "../../utils/displayName";
import { Donut } from "./Donut";
import styles from "./PieDrillDetail.module.css";

interface Props {
  /** 当前下钻的大类 slice（带 cats 列表） */
  slice: BreakdownSlice;
  /** 顶层总分钟数，用来算 slice 占比 */
  grandTotal: number;
  /** 当天 / 当周 / 当月的 AppUsage 列表，drill 内部按 categoryId filter + Top 5 */
  apps: AppUsage[];
  /** 小分类详情用，map cat color → category（其实 BreakdownSlice.cats 已带 color，这里只
   *  当作冗余兜底，不用也行；保留接口以备 RankedItem 等更多元数据接入） */
  cats: Category[];
  onBack: () => void;
}

const TOP_APPS_LIMIT = 5;

/**
 * 占比下钻视图：返回 + 缩小 Donut + 该大类的分类构成 + Top 5 应用。
 * 跟 PieView 互斥渲染（drillId 决定）。
 */
export function PieDrillDetail({ slice, grandTotal, apps, onBack, cats }: Props) {
  void cats; // 暂用 slice.cats 的 color/name 字段，cats 留作扩展口子
  const { t } = useTranslation();
  const fmtHM = useDurationFormatter();

  const sliceTotal = slice.minutes;
  const slicePct = grandTotal > 0 ? Math.round((sliceTotal / grandTotal) * 100) : 0;

  // 分类构成已经在 BreakdownSlice.cats 里按 minutes 降序；直接 map
  const catBreakdown = slice.cats;

  // Top N apps：从原始 AppUsage 按 categoryId 命中本 slice 的 cats 筛 + 降序
  const topApps = useMemo(() => {
    const supCatIds = new Set(slice.cats.map((c) => c.id));
    return apps
      .filter((a) => supCatIds.has(a.categoryId))
      .sort((a, b) => b.minutes - a.minutes)
      .slice(0, TOP_APPS_LIMIT);
  }, [apps, slice.cats]);

  const handleBack = () => withViewTransition(onBack);

  return (
    <div
      className={styles.body}
      style={{ viewTransitionName: "pie-body" } as CSSProperties}
    >
      <header className={styles.head}>
        <button
          type="button"
          className={styles.backBtn}
          onClick={handleBack}
        >
          <ArrowLeft size={12} strokeWidth={2.2} />
          {t("today.pie.drill.back")}
        </button>
        <span className={styles.sep} />
        <span
          className={styles.swatch}
          style={{ background: slice.color }}
          aria-hidden
        />
        <span className={styles.title}>{slice.name}</span>
        <div className={styles.headNumStack}>
          <span className={styles.headTime}>{fmtHM(sliceTotal)}</span>
          <span className={styles.headPct}>{slicePct}%</span>
        </div>
      </header>

      <div className={styles.split}>
        <div className={styles.donutWrap}>
          <Donut
            size={156}
            thickness={24}
            segments={[{ id: slice.id, color: slice.color, value: sliceTotal }]}
            total={grandTotal}
            activeId={slice.id}
            centerTitle={`${slicePct}%`}
            centerSub={slice.name}
            viewTransitionName="super-donut"
          />
        </div>

        <div className={styles.sections}>
          <section className={styles.section}>
            <header className={styles.sectionHead}>
              <h3 className={styles.sectionTitle}>
                {t("today.pie.drill.categoriesTitle")}
              </h3>
              <span className={styles.sectionCount}>{catBreakdown.length}</span>
            </header>
            <ul className={styles.drillList}>
              {catBreakdown.map((c) => {
                const pct = sliceTotal > 0 ? Math.round((c.minutes / sliceTotal) * 100) : 0;
                return (
                  <li key={c.id} className={styles.drillRow}>
                    <span
                      className={styles.drillSwatch}
                      style={{ background: c.color }}
                    />
                    <span className={styles.drillName}>{c.name}</span>
                    <span className={styles.drillBarWrap}>
                      <span
                        className={styles.drillBarFill}
                        style={{
                          width: `${pct}%`,
                          background: `color-mix(in oklab, ${c.color} 75%, transparent)`,
                        }}
                      />
                    </span>
                    <span className={styles.drillTime}>{fmtHM(c.minutes)}</span>
                  </li>
                );
              })}
            </ul>
          </section>

          <section className={styles.section}>
            <header className={styles.sectionHead}>
              <h3 className={styles.sectionTitle}>
                {t("today.pie.drill.appsTitle")}
              </h3>
              <span className={styles.sectionCount}>
                {t("today.pie.drill.topNCount", { count: topApps.length })}
              </span>
            </header>
            {topApps.length === 0 ? (
              <p className={styles.emptyApps}>—</p>
            ) : (
              <ul className={styles.drillList}>
                {topApps.map((a) => {
                  const pct = sliceTotal > 0 ? Math.round((a.minutes / sliceTotal) * 100) : 0;
                  return (
                    <li key={a.process} className={styles.drillRow}>
                      <span className={styles.appIconWrap}>
                        <AppIcon
                          processName={a.iconProcess}
                          fallbackColor={slice.color}
                          size={18}
                        />
                      </span>
                      <span className={styles.drillName}>
                        {displayAppName(a.process)}
                      </span>
                      <span className={styles.drillBarWrap}>
                        <span
                          className={styles.drillBarFill}
                          style={{
                            width: `${pct}%`,
                            background: `color-mix(in oklab, ${slice.color} 75%, transparent)`,
                          }}
                        />
                      </span>
                      <span className={styles.drillTime}>{fmtHM(a.minutes)}</span>
                    </li>
                  );
                })}
              </ul>
            )}
          </section>
        </div>
      </div>
    </div>
  );
}
