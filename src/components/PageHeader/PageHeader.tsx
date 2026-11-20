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
  /** tab 条右边的按钮，如进 AI 设置的齿轮 */
  actions?: ReactNode;
}

/**
 * 带 tab 的页面的标题行：标题在左，tab 条和按钮在右，一起占一行，省掉 tab 条单独一行的高度。
 * 一行放不下时右边整块换到标题下面。
 */
export function PageHeader({ title, tabs, groups, children, actions }: PageHeaderProps) {
  const hasTabs = Boolean(tabs || groups);
  return (
    <header className={styles.header} data-part="page-header">
      <div className={styles.text}>
        <h1 className={styles.title}>{title}</h1>
        {children}
      </div>
      {(hasTabs || actions) && (
        <div className={styles.end}>
          {hasTabs && (
            <div className={styles.tabs}>
              <FloatingTabNav tabs={tabs} groups={groups} ariaLabel={title} />
            </div>
          )}
          {actions}
        </div>
      )}
    </header>
  );
}
