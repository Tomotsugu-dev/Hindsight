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
  children: ReactNode;
}

export function Section({
  title,
  description,
  icon: Icon,
  tone = "primary",
  info,
  children,
}: SectionProps) {
  return (
    <section className={styles.section}>
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
      </header>
      <div className={styles.card}>{children}</div>
    </section>
  );
}
