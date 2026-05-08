import type { LucideIcon } from "lucide-react";
import { Info } from "lucide-react";
import type { ReactNode } from "react";
import styles from "./Section.module.css";

interface SectionProps {
  title: string;
  description?: string;
  icon?: LucideIcon;
  tone?: "primary" | "danger";
  /** 信息提示：传了就在标题右边渲染一个 info 小图标，hover 时浮出气泡。
   *  和 description 互斥使用——同时给的话 description 走标题下的常规位置，
   *  info 走 hover 气泡，两块文字并存。 */
  info?: string;
  /** 默认收起、hover / 内部 focus 时才展开。给"次要 / 不常改"的 Section 用，
   *  减少滚动条长度。展开走 grid-rows 0fr↔1fr 动画。 */
  collapsible?: boolean;
  /** Section 标题行最右侧的 action slot——给 Section 级别的"主操作"按钮用
   *  （例如「仅生成」、「全部清空」）。CSS 把它推到 header 行最右、跟标题对齐。 */
  headerAction?: ReactNode;
  children: ReactNode;
}

export function Section({
  title,
  description,
  icon: Icon,
  tone = "primary",
  info,
  collapsible = false,
  headerAction,
  children,
}: SectionProps) {
  return (
    <section
      className={`${styles.section} ${collapsible ? styles.collapsible : ""}`}
    >
      <header className={styles.header}>
        {Icon ? (
          <div className={`${styles.icon} ${styles[`tone_${tone}`]}`}>
            <Icon size={20} strokeWidth={1.85} />
          </div>
        ) : null}
        <div className={styles.headText}>
          <div className={styles.titleRow}>
            <h2 className={styles.title}>{title}</h2>
            {info ? (
              <span
                className={styles.infoWrap}
                tabIndex={0}
                aria-label={info}
              >
                <Info
                  size={14}
                  strokeWidth={1.85}
                  className={styles.infoIcon}
                />
                <span className={styles.infoTip} role="tooltip">
                  {info}
                </span>
              </span>
            ) : null}
          </div>
          {description ? (
            <p className={styles.description}>{description}</p>
          ) : null}
        </div>
        {headerAction ? (
          <div className={styles.headerAction}>{headerAction}</div>
        ) : null}
      </header>
      {/* collapsible 时外层 cardWrap 走 grid-rows trick；非 collapsible 直接渲 card */}
      {collapsible ? (
        <div className={styles.cardWrap}>
          <div className={styles.cardInner}>
            <div className={styles.card}>{children}</div>
          </div>
        </div>
      ) : (
        <div className={styles.card}>{children}</div>
      )}
    </section>
  );
}
