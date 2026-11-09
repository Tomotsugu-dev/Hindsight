import type { ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { ChevronLeft, ChevronRight } from "lucide-react";
import { Button } from "../../components/Button/Button";
import { IconButton } from "../../components/Button/IconButton";
import { DevicePicker } from "../../components/DevicePicker/DevicePicker";
import styles from "./HistoryChartCard.module.css";

export type ChartTab = "year" | "month";

const TABS: ChartTab[] = ["year", "month"];

/** 年表、月表头上的 ‹ 期间 › */
export interface PeriodNav {
  label: string;
  /** 已经是今年 / 本月时，中间的按钮不能点 */
  isCurrent: boolean;
  currentTooltip: string;
  prevLabel: string;
  nextLabel: string;
  canPrev: boolean;
  canNext: boolean;
  onPrev: () => void;
  onNext: () => void;
  onCurrent: () => void;
}

interface HistoryChartCardProps {
  title: string;
  /** 标题后面同一行的内容（全部历史放路径：全部 › 大类 › 应用 · 日期） */
  titleExtras?: ReactNode;
  tab: ChartTab;
  onTabChange: (tab: ChartTab) => void;
  nav: PeriodNav;
  children: ReactNode;
}

/** 全部历史的图表卡片，头部排法跟日、周、月统计的卡片一样：标题一行，标签和设备、期间一行。 */
export function HistoryChartCard({
  title,
  titleExtras,
  tab,
  onTabChange,
  nav,
  children,
}: HistoryChartCardProps) {
  const { t } = useTranslation();

  return (
    <section className={styles.card} data-part="card">
      <header className={styles.head}>
        <div className={styles.titleRow}>
          <h2 className={styles.title}>{title}</h2>
          {titleExtras}
        </div>
        <div className={styles.controls}>
          <div className={styles.tabs} role="tablist">
            <span
              className={styles.underline}
              style={{ transform: `translateX(${TABS.indexOf(tab) * 100}%)` }}
              aria-hidden
            />
            {TABS.map((key) => (
              <button
                key={key}
                type="button"
                role="tab"
                aria-selected={tab === key}
                className={`${styles.tab} ${tab === key ? styles.tabActive : ""}`}
                onClick={() => onTabChange(key)}
              >
                {t(`history.chartTabs.${key}`)}
              </button>
            ))}
          </div>

          <div className={styles.right}>
            <DevicePicker />
            <div className={styles.nav}>
              <IconButton
                icon={ChevronLeft}
                label={nav.prevLabel}
                variant="soft"
                size="sm"
                glow
                onClick={nav.onPrev}
                disabled={!nav.canPrev}
              />
              <Button
                size="sm"
                glow
                className={styles.pill}
                onClick={nav.onCurrent}
                disabled={nav.isCurrent}
                title={nav.isCurrent ? undefined : nav.currentTooltip}
              >
                {nav.label}
              </Button>
              <IconButton
                icon={ChevronRight}
                label={nav.nextLabel}
                variant="soft"
                size="sm"
                glow
                onClick={nav.onNext}
                disabled={!nav.canNext}
              />
            </div>
          </div>
        </div>
      </header>

      {children}
    </section>
  );
}
