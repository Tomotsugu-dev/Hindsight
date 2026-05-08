import { Info, type LucideIcon } from "lucide-react";
import type { ReactNode } from "react";
import styles from "./Row.module.css";

interface RowProps {
  label: string;
  description?: string;
  children: ReactNode;
  /** 是否禁用（视觉上灰显） */
  disabled?: boolean;
  /** 控件是否单独换行（用于较宽的控件，如时间段列表） */
  block?: boolean;
  /** 行首小图标 */
  icon?: LucideIcon;
  /** 图标色调 */
  tone?: "primary" | "danger";
  /** label 右侧 info 图标的 hover 提示（多行用 \n 分隔） */
  labelHint?: string;
}

export function Row({
  label,
  description,
  children,
  disabled,
  block,
  icon: Icon,
  tone = "primary",
  labelHint,
}: RowProps) {
  return (
    <div
      className={`${styles.row} ${block ? styles.rowBlock : ""} ${disabled ? styles.rowDisabled : ""}`}
    >
      <div className={styles.left}>
        {Icon ? (
          <div className={`${styles.icon} ${styles[`tone_${tone}`]}`}>
            <Icon size={16} strokeWidth={1.85} />
          </div>
        ) : null}
        <div className={styles.text}>
          <span className={styles.labelLine}>
            <span className={styles.label}>{label}</span>
            {labelHint ? (
              <button
                type="button"
                className={styles.infoWrap}
                aria-label={labelHint}
              >
                <Info
                  size={14}
                  strokeWidth={1.85}
                  className={styles.infoIcon}
                />
                <span className={styles.infoTip} role="tooltip">
                  {labelHint}
                </span>
              </button>
            ) : null}
          </span>
          {description ? (
            <span className={styles.description}>{description}</span>
          ) : null}
        </div>
      </div>
      <div className={styles.control}>{children}</div>
    </div>
  );
}
