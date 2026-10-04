import { useState } from "react";
import { Outlet } from "react-router-dom";
import { useTranslation } from "react-i18next";
import { type TabDef } from "../../components/TabNav/TabNav";
import { PageHeader } from "../../components/PageHeader/PageHeader";
import styles from "./HistoryPage.module.css";

/** 统计：图表和排行；搜索：屏幕上出现过的文字（原来在 AI 总结里） */
const TABS: TabDef[] = [
  { to: "", labelKey: "history.tabs.stats", end: true },
  { to: "search", labelKey: "history.tabs.search" },
];

/** 子页面通过 useOutletContext 拿到的东西 */
export interface HistoryOutletContext {
  /** 标题下面那行小字的位置；统计页把「共 X 小时 · 某天起」放进来，搜索页不放 */
  metaSlot: HTMLElement | null;
}

/** 全部历史页外壳：标题、小字和 tab 一行，下面是 Outlet。 */
export default function HistoryPage() {
  const { t } = useTranslation();
  const [metaSlot, setMetaSlot] = useState<HTMLElement | null>(null);

  return (
    <div className={styles.page}>
      <PageHeader title={t("history.title")} tabs={TABS}>
        <p className={styles.meta} ref={setMetaSlot} />
      </PageHeader>

      <section className={styles.tabContent}>
        <Outlet context={{ metaSlot } satisfies HistoryOutletContext} />
      </section>
    </div>
  );
}
