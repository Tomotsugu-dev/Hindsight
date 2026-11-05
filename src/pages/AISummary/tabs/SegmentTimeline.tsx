// 日报页和周报面板共用的时间轴：左列起止时间，中间色点连成一条线，右列段名和正文。

import type { HTMLAttributes, ReactNode } from "react";
import { hourLabel } from "./reportFormat";
import styles from "./SegmentTimeline.module.css";

/** 时间轴外框。各段的 `SegmentTimelineRow` 直接放在里面，竖线靠它们是兄弟节点来接上。 */
export function SegmentTimeline({ className, ...props }: HTMLAttributes<HTMLElement>) {
  return (
    <section
      className={className ? `${styles.timeline} ${className}` : styles.timeline}
      {...props}
    />
  );
}

interface SegmentTimelineRowProps {
  startHour: number;
  endHour: number;
  label: string;
  /** 圆点颜色，跟设置页同一份色源 */
  color: string;
  /** 空心点：这段没有内容 */
  hollow: boolean;
  /** 段名的标题级别，跟着外面最近的标题往下排 */
  heading: "h3" | "h4";
  /** 段名右边：生成中、未生成、重试按钮这类 */
  badge?: ReactNode;
  children?: ReactNode;
}

/** 时间轴上的一段。 */
export function SegmentTimelineRow({
  startHour,
  endHour,
  label,
  color,
  hollow,
  heading: Heading,
  badge,
  children,
}: SegmentTimelineRowProps) {
  return (
    <div className={styles.row}>
      <div className={styles.time}>
        <span className={styles.timeStart}>{hourLabel(startHour)}</span>
        <span className={styles.timeEnd}>{hourLabel(endHour)}</span>
      </div>
      <div className={styles.rail} aria-hidden>
        <span
          className={styles.dot}
          style={{ borderColor: color, background: hollow ? "transparent" : color }}
        />
      </div>
      <div className={styles.content}>
        <div className={styles.rowHead}>
          <Heading className={styles.label}>{label}</Heading>
          {badge}
        </div>
        {children}
      </div>
    </div>
  );
}
