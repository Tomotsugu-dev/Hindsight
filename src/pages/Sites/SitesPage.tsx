import { useCallback, useEffect, useMemo, useState, type ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { convertFileSrc } from "@tauri-apps/api/core";
import { Link } from "react-router-dom";
import { ArrowRight, ChevronDown, ChevronRight, Globe, Info, Search } from "lucide-react";
import youtubeIcon from "../../assets/site-icons/youtube.svg";
import { api, type SiteRow } from "../../api/hindsight";
import { ROUTES } from "../../config/nav";
import { logError } from "../../lib/logger";
import { useCategories } from "../../state/categories";
import { useSettings } from "../../state/settings";
import { ConfirmDialog } from "../../components/ConfirmDialog/ConfirmDialog";
import { Toggle } from "../../components/FormControls/Toggle";
import { CategoryFilterDropdown } from "../Apps/CategoryFilterDropdown";
import { SortDropdown } from "../Apps/AppsFilterBar";
import { useDurationFormatter } from "../../utils/duration";
import { AssignDropdown } from "../Categories/parts";
import categoriesStyles from "../Categories/Categories.module.css";
import filterBarStyles from "../Apps/AppsFilterBar.module.css";
import { filterGroups, groupSites, ruleSource, sortGroups, type SiteSortBy } from "./siteRows";
import { useSiteIcons } from "./useSiteIcons";
import styles from "./SitesPage.module.css";

const SORT_OPTIONS: SiteSortBy[] = [
  "default",
  "recentDesc",
  "recentAsc",
  "totalDesc",
  "totalAsc",
  "nameAsc",
  "nameDesc",
];

/** Opened groups, kept outside the component so they stay open after visiting another page. */
const openedGroups = new Set<string>();

/** A missing or unreadable cached file keeps the same globe placeholder as an undownloaded icon. */
function SiteIcon({ path }: { path?: string }) {
  const [failedPath, setFailedPath] = useState<string | null>(null);
  if (!path || path === failedPath) {
    return <Globe size={14} strokeWidth={2} className={styles.siteIcon} aria-hidden />;
  }
  return (
    <img
      className={styles.siteIcon}
      src={convertFileSrc(path)}
      width={16}
      height={16}
      alt=""
      aria-hidden
      draggable={false}
      onError={() => setFailedPath(path)}
    />
  );
}

/** A bundled example keeps the confirmation preview from contacting websites. */
function SiteIconPreview() {
  const { t } = useTranslation();
  return (
    <div className={styles.iconPreview} aria-label={t("categories.sites.icons.previewLabel")}>
      <div className={styles.previewState}>
        <span className={styles.previewLabel}>{t("categories.sites.icons.beforeDownload")}</span>
        <div className={styles.previewRow}>
          <Globe size={16} strokeWidth={2} className={styles.siteIcon} aria-hidden />
          <span className={styles.previewHost}>youtube.com</span>
        </div>
      </div>
      <ArrowRight size={16} className={styles.previewArrow} aria-hidden />
      <div className={styles.previewState}>
        <span className={styles.previewLabel}>{t("categories.sites.icons.afterDownload")}</span>
        <div className={styles.previewRow}>
          <img className={styles.siteIcon} src={youtubeIcon} width={16} height={16} alt="" />
          <span className={styles.previewHost}>youtube.com</span>
        </div>
      </div>
    </div>
  );
}

/**
 * Website classification tab (ADR-0013): assign a website to a category so that its
 * time in a browser counts toward that category instead of the browser's.
 */
export default function SitesPage() {
  const { t } = useTranslation();
  const { categories } = useCategories();
  const { settings, reload: reloadSettings } = useSettings();
  const formatDuration = useDurationFormatter();
  const [rows, setRows] = useState<SiteRow[] | null>(null);
  const [search, setSearch] = useState("");
  const [selectedCategoryIds, setSelectedCategoryIds] = useState<string[]>([]);
  const [unassignedOnly, setUnassignedOnly] = useState(false);
  const [sortBy, setSortBy] = useState<SiteSortBy>("default");
  const [opened, setOpened] = useState<ReadonlySet<string>>(() => new Set(openedGroups));
  const [iconConsentOpen, setIconConsentOpen] = useState(false);
  const [savingIcons, setSavingIcons] = useState(false);
  const [iconSaveFailed, setIconSaveFailed] = useState(false);

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

  const groups = useMemo(() => (rows ? groupSites(rows) : null), [rows]);
  const visibleGroups = useMemo(
    () =>
      groups
        ? sortGroups(filterGroups(groups, search, { selectedCategoryIds, unassignedOnly }), sortBy)
        : null,
    [groups, search, selectedCategoryIds, unassignedOnly, sortBy],
  );

  const toggleCategory = (id: string) => {
    setSelectedCategoryIds((selected) =>
      selected.includes(id) ? selected.filter((current) => current !== id) : [...selected, id],
    );
    setUnassignedOnly(false);
  };
  const resetCategories = () => {
    setSelectedCategoryIds([]);
    setUnassignedOnly(false);
  };
  const clearFilters = () => {
    setSearch("");
    resetCategories();
    setSortBy("default");
  };

  const iconHosts = useMemo(() => {
    if (!groups || !visibleGroups) return null;
    // Download the rows currently shown first, then the folded or filtered websites.
    const shown = visibleGroups.flatMap(({ group, children, forceOpen }) => [
      group.root.host,
      ...(forceOpen || opened.has(group.root.host) ? children.map((row) => row.host) : []),
    ]);
    const all = groups.flatMap((group) => [
      group.root.host,
      ...group.children.map((row) => row.host),
    ]);
    return [...new Set([...shown, ...all])];
  }, [groups, visibleGroups, opened]);
  const {
    icons,
    failed: iconLoadFailed,
    retry: retryIcons,
  } = useSiteIcons(iconHosts, settings?.downloadSiteIcons === true && !savingIcons);

  const saveIconDownloads = async (enabled: boolean) => {
    setIconConsentOpen(false);
    setSavingIcons(true);
    setIconSaveFailed(false);
    try {
      // Wait for persistence before starting downloads; the shared settings update is debounced.
      await api.updateSettings({ downloadSiteIcons: enabled });
      await reloadSettings();
    } catch (error) {
      logError("sites.icons.setting", error);
      setIconSaveFailed(true);
    } finally {
      setSavingIcons(false);
    }
  };

  const toggleGroup = (host: string) => {
    if (openedGroups.has(host)) openedGroups.delete(host);
    else openedGroups.add(host);
    setOpened(new Set(openedGroups));
  };

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

  // `lead` is the open/close button of a group, or an empty slot that keeps columns aligned.
  const siteRow = (
    row: SiteRow,
    even: boolean,
    lead: ReactNode,
    note: string | null,
    child: boolean,
  ) => {
    const source = ruleSource(row);
    const sub =
      note ?? (source === "parent" ? t("categories.sites.follows", { host: row.follows }) : null);
    return (
      <div key={row.host} className={`${styles.row} ${even ? styles.rowEven : ""}`}>
        <div className={`${styles.siteCol} ${child ? styles.childSite : ""}`}>
          {lead}
          <SiteIcon path={icons[row.host]} />
          <div className={styles.siteText}>
            <span className={styles.host} title={row.host}>
              {row.host}
            </span>
            {sub && <span className={styles.follows}>{sub}</span>}
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
  };

  const emptySlot = <span className={styles.toggle} aria-hidden />;
  let rowIndex = 0;
  const tableRows = (visibleGroups ?? []).flatMap(({ group, children, forceOpen }) => {
    const { root } = group;
    const hasChildren = children.length > 0;
    const open = forceOpen || opened.has(root.host);
    const lead = hasChildren ? (
      <button
        type="button"
        className={styles.toggle}
        aria-expanded={open}
        aria-label={t(open ? "categories.sites.collapse" : "categories.sites.expand")}
        disabled={forceOpen}
        onClick={() => toggleGroup(root.host)}
      >
        {open ? (
          <ChevronDown size={14} strokeWidth={2} />
        ) : (
          <ChevronRight size={14} strokeWidth={2} />
        )}
      </button>
    ) : (
      emptySlot
    );
    // A closed group says what it hides: how many subdomains, and how many of them count
    // toward another category than the one shown.
    const note =
      hasChildren && !open
        ? [
            t("categories.sites.subdomains", { count: children.length }),
            children.some((child) => child.categoryId !== root.categoryId)
              ? t("categories.sites.otherCategory", {
                  count: children.filter((child) => child.categoryId !== root.categoryId).length,
                })
              : null,
          ]
            .filter(Boolean)
            .join(" · ")
        : null;
    const lines = [siteRow(root, rowIndex++ % 2 === 0, lead, note, false)];
    if (open) {
      for (const c of children) lines.push(siteRow(c, rowIndex++ % 2 === 0, emptySlot, null, true));
    }
    return lines;
  });

  return (
    <>
      <header className={`${categoriesStyles.header} ${styles.header}`}>
        <div className={`${categoriesStyles.headerText} ${styles.headerText}`}>
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
        <div className={styles.iconSetting}>
          <span>{t("categories.sites.icons.label")}</span>
          <Toggle
            checked={settings?.downloadSiteIcons === true}
            disabled={!settings || savingIcons}
            ariaLabel={t("categories.sites.icons.label")}
            onChange={(enabled) => {
              if (enabled) setIconConsentOpen(true);
              else void saveIconDownloads(false);
            }}
          />
        </div>
      </header>

      <div className={filterBarStyles.bar}>
        <div className={`${filterBarStyles.searchWrap} ${styles.siteSearch}`}>
          <Search size={14} strokeWidth={2} className={filterBarStyles.searchIcon} aria-hidden />
          <input
            className={filterBarStyles.searchInput}
            type="text"
            value={search}
            onChange={(e) => setSearch(e.target.value)}
            placeholder={t("categories.sites.searchPlaceholder")}
            aria-label={t("categories.sites.searchPlaceholder")}
            spellCheck={false}
          />
        </div>
        <div className={`${filterBarStyles.rightGroup} ${styles.filterActions}`}>
          <CategoryFilterDropdown
            categories={categories}
            selectedCategoryIds={selectedCategoryIds}
            unassignedOnly={unassignedOnly}
            onToggleCategory={toggleCategory}
            onToggleUnassigned={() => {
              setUnassignedOnly((current) => !current);
              setSelectedCategoryIds([]);
            }}
            onReset={resetCategories}
          />
          <SortDropdown
            value={sortBy}
            onChange={setSortBy}
            label={t("apps.filter.sortLabel")}
            options={SORT_OPTIONS.map((value) => ({
              value,
              label: t(`categories.sites.sort.${value}`),
            }))}
          />
        </div>
      </div>
      {iconSaveFailed && (
        <p className={styles.iconError} role="alert">
          {t("categories.sites.icons.saveFailed")}
        </p>
      )}
      {iconLoadFailed && (
        <p className={styles.iconError} role="status">
          {t("categories.sites.icons.loadFailed")}{" "}
          <button type="button" className={styles.retryIcons} onClick={retryIcons}>
            {t("categories.sites.icons.retry")}
          </button>
        </p>
      )}

      <section className={categoriesStyles.card}>
        {rows === null || visibleGroups === null ? (
          <div className={styles.status}>{t("categories.sites.loading")}</div>
        ) : rows.length === 0 ? (
          <div className={styles.status}>
            {t("categories.sites.empty")}{" "}
            <Link to={`${ROUTES.settings}/privacy`} className={styles.statusLink}>
              {t("categories.sites.openPrivacy")}
            </Link>
          </div>
        ) : visibleGroups.length === 0 ? (
          <div className={filterBarStyles.empty}>
            {t(
              unassignedOnly && !search.trim()
                ? "categories.sites.allAssigned"
                : "categories.sites.noResults",
            )}
            <button type="button" className={filterBarStyles.emptyClearBtn} onClick={clearFilters}>
              {t("apps.filter.clearFilters")}
            </button>
          </div>
        ) : (
          <div className={styles.table}>
            <div className={styles.headerRow}>
              <span>
                {t("categories.sites.header.site")}
                <span className={styles.domainCount}>
                  {t("categories.sites.header.domainCount", { count: rows.length })}
                </span>
              </span>
              <span className={styles.alignEnd}>{t("categories.sites.header.last30d")}</span>
              <span className={styles.alignEnd}>{t("categories.sites.header.total")}</span>
              <span>{t("categories.sites.header.category")}</span>
            </div>
            {tableRows}
          </div>
        )}
      </section>

      <ConfirmDialog
        open={iconConsentOpen}
        title={t("categories.sites.icons.confirmTitle")}
        message={
          <>
            <SiteIconPreview />
            <p className={styles.trafficNote}>{t("categories.sites.icons.trafficNote")}</p>
          </>
        }
        confirmLabel={t("categories.sites.icons.confirmAction")}
        onConfirm={() => void saveIconDownloads(true)}
        onCancel={() => setIconConsentOpen(false)}
      />
    </>
  );
}
