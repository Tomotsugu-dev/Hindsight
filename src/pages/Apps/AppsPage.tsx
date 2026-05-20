import { useCallback, useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { Info } from "lucide-react";
import { api, type AppGroup } from "../../api/hindsight";
import { logError } from "../../lib/logger";
import { useCategories } from "../../state/categories";
import { PairingSection } from "../Categories/PairingSection";
import categoriesStyles from "../Categories/Categories.module.css";
import { AppsFilterBar } from "./AppsFilterBar";
import { applyFilter } from "./filterPipeline";
import { useAppsFilter } from "./useAppsFilter";
import filterBarStyles from "./AppsFilterBar.module.css";

/** 行数 ≤ 这个阈值时不显示工具栏（新用户没几个 app，工具栏纯打扰）。 */
const TOOLBAR_THRESHOLD = 10;

/**
 * 应用页：跨设备应用合并 + 分类指派。
 *
 * 同一应用在不同 OS 上进程名不同（macOS 的 "Code" / Windows 的 "Visual Studio
 * Code"）—— 这页把它们拖到同一行就统一名字 / 分类 / 图标 / 时长。
 *
 * 数据流：AppsPage 持有 AppGroup 全量 fetch + 筛选状态（[`useAppsFilter`]），把过滤后的
 * groups 传给 [PairingSection](../Categories/PairingSection.tsx)（受控模式）。
 */
export default function AppsPage() {
  const { t } = useTranslation();
  const { categories, refresh: refreshCategories } = useCategories();
  const [groups, setGroups] = useState<AppGroup[] | null>(null);

  const filter = useAppsFilter();

  const reload = useCallback(async () => {
    try {
      const list = await api.listAppGroups();
      setGroups(list);
    } catch (e) {
      logError("apps.listGroups", e);
      setGroups([]);
    }
  }, []);

  useEffect(() => {
    void reload();
  }, [reload]);

  const onCreateRow = useCallback(async () => {
    const time = new Date().toLocaleTimeString(undefined, {
      hour: "2-digit",
      minute: "2-digit",
    });
    const defaultName = t("categories.pairing.newRowDefaultName", { time });
    try {
      await api.createAppGroup(defaultName);
      await reload();
    } catch (e) {
      logError("apps.createRow", e);
    }
  }, [reload, t]);

  // 过滤后用于渲染的 groups；空数组占位避免 .map 上 null
  const filteredGroups = useMemo<AppGroup[] | null>(() => {
    if (groups === null) return null;
    return applyFilter(groups, filter.filter);
  }, [groups, filter.filter]);

  const totalGroupsCount = groups?.length ?? 0;
  const showToolbar = totalGroupsCount > TOOLBAR_THRESHOLD;

  // PairingSection 受控模式：传入过滤后的 groups + 共享 reload；同时让它
  // 把 merge/unmerge/rename/delete 完成后的刷新转发回我们（保持 AppsPage 是数据 owner）
  const onPairingReload = useCallback(async () => {
    await Promise.all([reload(), refreshCategories()]);
  }, [reload, refreshCategories]);

  return (
    <div className={categoriesStyles.page}>
      <header className={categoriesStyles.header}>
        <div className={categoriesStyles.headerText}>
          <h1 className={categoriesStyles.title}>
            {t("apps.title")}
            <button
              type="button"
              className={categoriesStyles.infoTip}
              aria-label={t("categories.pairing.infoTipAria")}
            >
              <Info size={14} strokeWidth={2.25} />
              <span className={categoriesStyles.infoTipBody} role="tooltip">
                {t("categories.pairing.infoTipBody")}
              </span>
            </button>
          </h1>
          <p className={categoriesStyles.meta}>
            {t("categories.pairing.instructionPrefix")}
            <strong className={categoriesStyles.metaEmph}>
              {t("categories.pairing.instructionEmph")}
            </strong>
            {t("categories.pairing.instructionSuffix")}
            <span className={categoriesStyles.metaUnassigned}>
              {t("categories.pairing.unassignedHint")}
            </span>
          </p>
        </div>
      </header>

      {showToolbar && (
        <AppsFilterBar
          search={filter.filter.search}
          onSearchChange={filter.setSearch}
          categories={categories}
          selectedCategoryIds={filter.filter.selectedCategoryIds}
          unassignedOnly={filter.filter.unassignedOnly}
          onToggleCategory={filter.toggleCategory}
          onToggleUnassigned={filter.toggleUnassignedOnly}
          onResetCategories={filter.resetCategories}
          sortBy={filter.filter.sortBy}
          onSortChange={filter.setSortBy}
          onCreateRow={() => void onCreateRow()}
        />
      )}

      <section className={categoriesStyles.card}>
        {/* 筛后空态：原始数据存在但 filter 后为 0 行 → 显示空态 + 清除筛选按钮。
            未筛任何东西却空的（用户全新装机，groups.length=0）→ 走 PairingSection
            内部的"无设备"/"无数据"提示，不抢戏。 */}
        {filteredGroups && filteredGroups.length === 0 && filter.isFiltering ? (
          <div className={filterBarStyles.empty}>
            {t("apps.filter.noResults")}
            <button
              type="button"
              className={filterBarStyles.emptyClearBtn}
              onClick={filter.clearAll}
            >
              {t("apps.filter.clearFilters")}
            </button>
          </div>
        ) : (
          <PairingSection
            groups={filteredGroups ?? undefined}
            loading={filteredGroups === null}
            onReload={onPairingReload}
            showNewRowButton={!showToolbar}
            onCreateRow={onCreateRow}
          />
        )}
      </section>
    </div>
  );
}
