// 日报、周报共用的控件：页面顶部（副标题、时段切换、操作按钮）、内容框里的状态行和提示条。
// 样式在 ReportControls.module.css；占位条、错误框这类正文里的样式也在那里，各 tab 直接引用。
import type { ReactNode } from "react";
import { AlertTriangle, Check, Loader2, Play, RefreshCcw, Square, X } from "lucide-react";
import { Button } from "../../../components/Button/Button";
import { IconButton } from "../../../components/Button/IconButton";
import styles from "./ReportControls.module.css";

interface ReportHeaderProps {
  subtitle: string;
  /** 左边：时段切换 */
  nav: ReactNode;
  /** 右边：主按钮、导出、删除 */
  actions: ReactNode;
}

/** 页面顶部：副标题、工具栏。 */
export function ReportHeader({ subtitle, nav, actions }: ReportHeaderProps) {
  return (
    <div className={styles.top}>
      <p className={styles.subtitle}>{subtitle}</p>
      <header className={styles.toolbar}>
        {nav}
        <div className={styles.actions}>{actions}</div>
      </header>
    </div>
  );
}

interface ReportStatusProps {
  /** 正在看的具体日期，如「11月5日周四」；切换按钮上只写相对时间，日期写在这里 */
  period: string;
  /** 生成到哪了、什么时候生成的、用的模型；busy 时前面带转圈，空串时只写日期 */
  status: { busy: boolean; text: string };
  className?: string;
}

/** 内容框顶上那行小字：日期和这份报告的生成情况。 */
export function ReportStatus({ period, status, className }: ReportStatusProps) {
  return (
    <div
      className={className ? `${styles.statusLine} ${className}` : styles.statusLine}
      aria-live="polite"
    >
      <span className={styles.statusPeriod}>{period}</span>
      {status.busy || status.text ? (
        <span className={styles.statusSep} aria-hidden>
          ·
        </span>
      ) : null}
      {status.busy ? <Loader2 size={14} className={`${styles.spin} ${styles.statusSpin}`} /> : null}
      {status.text ? <span>{status.text}</span> : null}
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
          <IconButton
            size="xs"
            tone="danger"
            icon={X}
            label={dismissLabel}
            onClick={onDismissError}
          />
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
