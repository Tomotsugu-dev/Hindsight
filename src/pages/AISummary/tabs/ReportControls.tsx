// 日报、周报共用的控件：页面顶部（副标题、时段切换、操作按钮、状态行）和提示条。
// 样式在 ReportControls.module.css；占位条、错误框这类正文里的样式也在那里，各 tab 直接引用。
import type { ReactNode } from "react";
import {
  AlertTriangle,
  Check,
  ChevronLeft,
  ChevronRight,
  Loader2,
  Play,
  RefreshCcw,
  Square,
  X,
} from "lucide-react";
import { Button } from "../../../components/Button/Button";
import { IconButton } from "../../../components/Button/IconButton";
import styles from "./ReportControls.module.css";

interface ReportHeaderProps {
  subtitle: string;
  /** 左边：时段切换 */
  nav: ReactNode;
  /** 右边：主按钮、导出、删除 */
  actions: ReactNode;
  /** 正在看的具体日期，如「11月5日周四」；切换按钮上只写相对时间，日期写在这里 */
  period: string;
  /** 工具栏下面那行小字；busy 时前面带转圈 */
  status: { busy: boolean; text: string };
}

/** 页面顶部：副标题、工具栏、状态行。 */
export function ReportHeader({ subtitle, nav, actions, period, status }: ReportHeaderProps) {
  return (
    <div className={styles.top}>
      <p className={styles.subtitle}>{subtitle}</p>
      <header className={styles.toolbar}>
        {nav}
        <div className={styles.actions}>{actions}</div>
      </header>
      <div className={styles.statusLine} aria-live="polite">
        <span className={styles.statusPeriod}>{period}</span>
        <span className={styles.statusSep} aria-hidden>
          ·
        </span>
        {status.busy ? (
          <Loader2 size={14} className={`${styles.spin} ${styles.statusSpin}`} />
        ) : null}
        <span>{status.text}</span>
      </div>
    </div>
  );
}

interface PeriodNavProps {
  /** 整个控件的读屏名字，如「选择日期」 */
  groupLabel: string;
  prevLabel: string;
  nextLabel: string;
  /** 中间按钮的提示，如「回到今天」 */
  resetTitle: string;
  /** 中间按钮的字：「今天」「昨天」「3 天前」，跟日统计一样只写相对时间 */
  label: string;
  /** 已经是当前时段：中间按钮和「›」点不了 */
  atCurrent: boolean;
  /** 正在生成：不许切换 */
  locked: boolean;
  onPrev: () => void;
  onNext: () => void;
  onReset: () => void;
}

/** 时段切换：[‹] [今天] [›]，点中间回到当前时段。样式照日统计的日期切换。 */
export function PeriodNav(p: PeriodNavProps) {
  const canReset = !p.atCurrent;
  return (
    <div className={styles.periodNav} role="group" aria-label={p.groupLabel}>
      <IconButton
        variant="soft"
        size="sm"
        glow
        icon={ChevronLeft}
        label={p.prevLabel}
        onClick={p.onPrev}
        disabled={p.locked}
      />
      <Button
        variant="soft"
        size="sm"
        glow
        className={styles.navPill}
        onClick={p.onReset}
        disabled={p.locked || !canReset}
        title={canReset ? p.resetTitle : undefined}
      >
        {p.label}
      </Button>
      <IconButton
        variant="soft"
        size="sm"
        glow
        icon={ChevronRight}
        label={p.nextLabel}
        onClick={p.onNext}
        disabled={p.locked || p.atCurrent}
      />
    </div>
  );
}

interface MainButtonProps {
  action: "start" | "regenerate" | "stop";
  label: string;
  title?: string;
  disabled?: boolean;
  onClick: () => void;
}

/** 主按钮的轻重跟着状态走：没生成时实心，生成完浅底，在跑时中性。 */
export function MainButton({ action, label, title, disabled, onClick }: MainButtonProps) {
  return (
    <Button
      size="lg"
      variant={action === "start" ? "solid" : "soft"}
      tone={action === "stop" ? "neutral" : "accent"}
      icon={action === "start" ? Play : action === "regenerate" ? RefreshCcw : Square}
      onClick={onClick}
      disabled={disabled}
      title={title}
    >
      {label}
    </Button>
  );
}

interface NoticeBarsProps {
  error: string | null;
  dismissLabel: string;
  onDismissError: () => void;
  /** 导出成功这类短暂提示 */
  notice: string | null;
}

/** 顶部的错误条和成功条，没有内容时什么都不画。 */
export function NoticeBars({ error, dismissLabel, onDismissError, notice }: NoticeBarsProps) {
  return (
    <>
      {error ? (
        <div className={styles.errorBar}>
          <AlertTriangle size={14} strokeWidth={2.2} />
          <span>{error}</span>
          <IconButton size="sm" icon={X} label={dismissLabel} onClick={onDismissError} />
        </div>
      ) : null}
      {notice ? (
        <div className={styles.successBar}>
          <Check size={14} strokeWidth={2.4} />
          <span>{notice}</span>
        </div>
      ) : null}
    </>
  );
}
