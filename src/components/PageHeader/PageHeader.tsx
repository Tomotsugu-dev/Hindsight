import type { ReactNode } from "react";
import { type TabDef } from "../TabNav/TabNav";
import { FloatingTabNav } from "../TabNav/FloatingTabNav";
import styles from "./PageHeader.module.css";

interface PageHeaderProps {
  title: string;
  /** 标题右边的 tab 条；传 groups 时分组之间画竖线（见 TabNav） */
  tabs?: TabDef[];
  groups?: TabDef[][];
  /** 标题下面的内容，一般是一行小字 */
  children?: ReactNode;
}

/**
 * 带 tab 的页面的标题行：标题在左，tab 条在右，一起占一行，省掉 tab 条单独一行的高度。
 * 一行放不下时 tab 条换到标题下面。
 */
export function PageHeader({ title, tabs, groups, children }: PageHeaderProps) {
  const hasTabs = Boolean(tabs || groups);
  return (
    <header className={styles.header}>
      <div className={styles.text}>
        <h1 className={styles.title}>{title}</h1>
        {children}
      </div>
      {hasTabs && (
        <div className={styles.tabs}>
          <FloatingTabNav tabs={tabs} groups={groups} ariaLabel={title} />
        </div>
      )}
    </header>
  );
}
