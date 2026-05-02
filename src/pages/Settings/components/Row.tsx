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
}

export function Row({ label, description, children, disabled, block }: RowProps) {
  return (
    <div
      className={`${styles.row} ${block ? styles.rowBlock : ""} ${disabled ? styles.rowDisabled : ""}`}
    >
      <div className={styles.text}>
        <span className={styles.label}>{label}</span>
        {description ? <span className={styles.description}>{description}</span> : null}
      </div>
      <div className={styles.control}>{children}</div>
    </div>
  );
}
