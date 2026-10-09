import { useCallback, useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { Link } from "react-router-dom";
import { Globe, Info, Search } from "lucide-react";
import { api, type SiteRow } from "../../api/hindsight";
import { ROUTES } from "../../config/nav";
import { logError } from "../../lib/logger";
import { useCategories } from "../../state/categories";
import { useDurationFormatter } from "../../utils/duration";
import { AssignDropdown } from "../Categories/parts";
import categoriesStyles from "../Categories/Categories.module.css";
import filterBarStyles from "../Apps/AppsFilterBar.module.css";
import { filterSites, ruleSource } from "./siteRows";
import styles from "./SitesPage.module.css";

/** The search box shows only when there are more websites than this. */
const SEARCH_THRESHOLD = 10;

/**
 * Website classification tab (ADR-0013): assign a website to a category so that its
 * time in a browser counts toward that category instead of the browser's.
 */
export default function SitesPage() {
  const { t } = useTranslation();
  const { categories } = useCategories();
  const formatDuration = useDurationFormatter();
  const [rows, setRows] = useState<SiteRow[] | null>(null);
  const [search, setSearch] = useState("");

  const reload = useCallback(async () => {
    try {
      setRows(await api.listSites());
    } catch (e) {
      logError("sites.list", e);
      setRows([]);
    }
  }, []);

  useEffect(() => {
    void reload();
  }, [reload]);

  const visibleRows = useMemo(() => (rows ? filterSites(rows, search) : null), [rows, search]);

  // A rule on a parent domain changes its subdomains too, so reload the whole list.
  const onPick = async (host: string, categoryId: string | null) => {
    try {
      if (categoryId === null) {
        await api.removeSiteRule(host);
      } else {
        await api.setSiteRule(host, categoryId);
      }
    } catch (e) {
      logError("sites.setRule", e);
    }
    await reload();
  };

  return (
    <>
      <header className={categoriesStyles.header}>
        <div className={categoriesStyles.headerText}>
          <p className={categoriesStyles.meta}>
            {t("categories.sites.intro")}
            <button
              type="button"
              className={categoriesStyles.infoTip}
              aria-label={t("categories.sites.infoTipAria")}
            >
              <Info size={14} strokeWidth={2.25} />
              <span className={categoriesStyles.infoTipBody} role="tooltip">
                {t("categories.sites.infoTipBody")}
              </span>
            </button>
          </p>
        </div>
      </header>

      {rows !== null && rows.length > SEARCH_THRESHOLD && (
        <div className={filterBarStyles.bar}>
          <div className={filterBarStyles.searchWrap}>
            <Search size={14} strokeWidth={2} className={filterBarStyles.searchIcon} aria-hidden />
            <input
              className={filterBarStyles.searchInput}
              type="text"
              value={search}
              onChange={(e) => setSearch(e.target.value)}
              placeholder={t("categories.sites.searchPlaceholder")}
              spellCheck={false}
            />
          </div>
        </div>
      )}

      <section className={categoriesStyles.card}>
        {rows === null || visibleRows === null ? (
          <div className={styles.status}>{t("categories.sites.loading")}</div>
        ) : rows.length === 0 ? (
          <div className={styles.status}>
            {t("categories.sites.empty")}{" "}
            <Link to={`${ROUTES.settings}/privacy`} className={styles.statusLink}>
              {t("categories.sites.openPrivacy")}
            </Link>
          </div>
        ) : visibleRows.length === 0 ? (
          <div className={filterBarStyles.empty}>
            {t("categories.sites.noResults")}
            <button
              type="button"
              className={filterBarStyles.emptyClearBtn}
              onClick={() => setSearch("")}
            >
              {t("categories.sites.clearSearch")}
            </button>
          </div>
        ) : (
          <div className={styles.table}>
            <div className={styles.headerRow}>
              <span>{t("categories.sites.header.site")}</span>
              <span className={styles.alignEnd}>{t("categories.sites.header.last30d")}</span>
              <span className={styles.alignEnd}>{t("categories.sites.header.total")}</span>
              <span>{t("categories.sites.header.category")}</span>
            </div>
            {visibleRows.map((row, idx) => {
              const source = ruleSource(row);
              return (
                <div
                  key={row.host}
                  className={`${styles.row} ${idx % 2 === 0 ? styles.rowEven : ""}`}
                >
                  <div className={styles.siteCol}>
                    <Globe size={14} strokeWidth={2} className={styles.siteIcon} aria-hidden />
                    <div className={styles.siteText}>
                      <span className={styles.host} title={row.host}>
                        {row.host}
                      </span>
                      {source === "parent" && (
                        <span className={styles.follows}>
                          {t("categories.sites.follows", { host: row.follows })}
                        </span>
                      )}
                    </div>
                  </div>
                  <span className={styles.num}>{formatDuration(row.minutes30d)}</span>
                  <span className={styles.num}>{formatDuration(row.minutesTotal)}</span>
                  <div className={styles.categoryCol}>
                    <AssignDropdown
                      categories={categories}
                      currentCategoryId={row.categoryId}
                      // Only a website's own rule can be removed; an inherited category
                      // is changed by giving the website its own rule.
                      allowClear={source === "own"}
                      clearLabel={t("categories.sites.removeRule")}
                      onPick={(categoryId) => onPick(row.host, categoryId)}
                    />
                  </div>
                </div>
              );
            })}
          </div>
        )}
      </section>
    </>
  );
}
