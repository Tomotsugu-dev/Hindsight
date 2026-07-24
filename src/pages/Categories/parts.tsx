import { useEffect, useMemo, useRef, useState, type CSSProperties } from "react";
import { createPortal } from "react-dom";
import { useTranslation } from "react-i18next";
import { ChevronDown, Plus, X } from "lucide-react";
import { useCategories } from "../../state/categories";
import { api, type AppGroup, type Category } from "../../api/hindsight";
import { AppIcon } from "../../components/AppIcon/AppIcon";
import { resolveCategoryIcon } from "../../config/categoryIcons";
import { displayAppName } from "../../utils/displayName";
import { displayCategoryName } from "../../utils/categoryName";
import { logError } from "../../lib/logger";
import styles from "./Categories.module.css";

/** 自动补全候选:Hindsight 记录过的一个进程 + 它的归属信息。 */
interface AppSuggestion {
  process: string;
  display: string;
  /** 所在应用组的显示名(与 display 不同时作为附加匹配字段) */
  groupName: string;
  /** 现属分类 id;null = 未分类 */
  categoryId: string | null;
  /** 近 7 天用时,用作默认排序(常用的排前面) */
  recentSecs: number;
}

/** 面板最多渲染的候选数(超出滚动)。 */
const SUGGEST_MAX = 8;

export function AppList({ category }: { category: Category }) {
  const { t } = useTranslation();
  const { categories, unassignApp, assignApp } = useCategories();
  const [adding, setAdding] = useState(false);
  const [draft, setDraft] = useState("");
  // 自动补全:候选池(展开输入框时懒加载一次)、高亮下标(-1 = 无高亮,
  // Enter 提交输入原文——候选之外的自由输入能力保留)
  const [groups, setGroups] = useState<AppGroup[] | null>(null);
  const [hi, setHi] = useState(-1);
  // 面板用 fixed 定位挂在输入框下方:appList 容器有 overflow:hidden
  // (折叠动画),absolute 面板会被剪,与 AssignDropdown 同款处理
  const [menuRect, setMenuRect] = useState<{ top: number; left: number } | null>(null);
  const inputRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    if (!adding) {
      setMenuRect(null);
      return;
    }
    inputRef.current?.focus();
    if (groups === null) {
      api
        .listAppGroups()
        .then(setGroups)
        .catch((e) => {
          logError("categories.appSuggest", e);
          setGroups([]); // 拉不到候选就退化为纯手输,不挡添加
        });
    }
    const measure = () => {
      const r = inputRef.current?.getBoundingClientRect();
      if (r) setMenuRect({ top: r.bottom + 4, left: r.left });
    };
    measure();
    window.addEventListener("scroll", measure, true);
    window.addEventListener("resize", measure);
    return () => {
      window.removeEventListener("scroll", measure, true);
      window.removeEventListener("resize", measure);
    };
  }, [adding, groups]);

  // 候选池:全部已记录进程,按近 7 天用时降序;排除已在本分类的
  const pool = useMemo<AppSuggestion[]>(() => {
    if (!groups) return [];
    const inThis = new Set(category.apps);
    return groups
      .flatMap((g) =>
        g.members.map((m) => ({
          process: m.processName,
          display: displayAppName(m.processName),
          groupName: g.displayName,
          categoryId: g.categoryId,
          recentSecs: m.recentSecs,
        })),
      )
      .filter((s) => !inThis.has(s.process))
      // 未分类优先(添加应用的最高频场景是收编新应用),组内再按近 7 天用时;
      // 已分类的靠输入过滤依然可选(选中即挪移,右侧有现属分类标注)
      .sort(
        (a, b) =>
          Number(a.categoryId !== null) - Number(b.categoryId !== null) ||
          b.recentSecs - a.recentSecs,
      );
  }, [groups, category.apps]);

  // 输入过滤:进程名 / 显示名 / 组名 子串匹配,忽略大小写
  const matches = useMemo(() => {
    const q = draft.trim().toLowerCase();
    const hit = q
      ? pool.filter(
          (s) =>
            s.process.toLowerCase().includes(q) ||
            s.display.toLowerCase().includes(q) ||
            s.groupName.toLowerCase().includes(q),
        )
      : pool;
    return hit.slice(0, SUGGEST_MAX);
  }, [pool, draft]);

  // 输入变化后高亮回到"无",避免指到过滤后错位的项
  useEffect(() => {
    setHi(-1);
  }, [draft]);

  const catOf = (id: string | null) =>
    id ? categories.find((x) => x.id === id) ?? null : null;

  const commitName = async (name: string) => {
    const trimmed = name.trim();
    if (trimmed) {
      await assignApp(trimmed, category.id);
    }
    setDraft("");
    setAdding(false);
    setHi(-1);
  };

  const commit = () => commitName(hi >= 0 && matches[hi] ? matches[hi].process : draft);

  const cancel = () => {
    setDraft("");
    setAdding(false);
    setHi(-1);
  };

  return (
    <div className={styles.appList}>
      {category.apps.length === 0 && !adding && (
        <span className={styles.empty} style={{ padding: 0 }}>
          {t("categories.appList.empty")}
        </span>
      )}
      {category.apps.map((app) => {
        const display = displayAppName(app);
        return (
          <span key={app} className={styles.appChip}>
            <AppIcon
              processName={app}
              fallbackColor={category.color}
              size={14}
            />
            {display}
            <button
              type="button"
              className={styles.appChipRemove}
              onClick={() => unassignApp(app)}
              aria-label={t("categories.appList.removeAria", { name: display })}
              title={t("categories.appList.removeTooltip")}
            >
              <X size={10} strokeWidth={2.25} />
            </button>
          </span>
        );
      })}
      {adding ? (
        <span className={styles.appSuggestWrap}>
          <input
            ref={inputRef}
            className={styles.appAddInput}
            placeholder={t("categories.appList.addPlaceholder")}
            value={draft}
            maxLength={64}
            onChange={(e) => setDraft(e.target.value)}
            // blur 只按输入原文提交(旧语义):悬停候选设过高亮,点击别处
            // 不该把悬停项误加进来;选候选走键盘 Enter 或选项 mousedown
            onBlur={() => void commitName(draft)}
            onKeyDown={(e) => {
              if (e.nativeEvent.isComposing) return; // 输入法组词中不截获
              if (e.key === "ArrowDown") {
                e.preventDefault();
                setHi((v) => (matches.length ? (v + 1) % matches.length : -1));
              } else if (e.key === "ArrowUp") {
                e.preventDefault();
                setHi((v) =>
                  matches.length ? (v - 1 + matches.length) % matches.length : -1,
                );
              } else if (e.key === "Enter") {
                void commit();
              } else if (e.key === "Escape") {
                cancel();
              }
            }}
          />
          {matches.length > 0 &&
            menuRect &&
            createPortal(
              <div
                className={styles.appSuggestMenu}
                role="listbox"
                style={{ top: menuRect.top, left: menuRect.left }}
              >
              {matches.map((s, i) => {
                const cat = catOf(s.categoryId);
                const CatIcon = cat ? resolveCategoryIcon(cat.icon) : null;
                return (
                  <button
                    key={s.process}
                    type="button"
                    role="option"
                    aria-selected={i === hi}
                    className={`${styles.appSuggestOption} ${
                      i === hi ? styles.appSuggestOptionHi : ""
                    }`}
                    // mousedown + preventDefault:先于输入框 blur 落点,
                    // 避免 blur 把半截输入原文提交出去
                    onMouseDown={(e) => {
                      e.preventDefault();
                      void commitName(s.process);
                    }}
                    onMouseEnter={() => setHi(i)}
                  >
                    <AppIcon
                      processName={s.process}
                      fallbackColor={category.color}
                      size={14}
                    />
                    <span className={styles.appSuggestLabel}>{s.display}</span>
                    {/* 未分类 = 该收编的:accent 小圆点;已分类的显示现属分类
                        (图标染分类色 + 灰字名,与 AssignDropdown 同款视觉) */}
                    {s.categoryId === null ? (
                      <span className={styles.appSuggestDot} aria-hidden />
                    ) : (
                      cat &&
                      CatIcon && (
                        <span
                          className={styles.appSuggestMeta}
                          style={{ "--cat-color": cat.color } as CSSProperties}
                        >
                          <CatIcon
                            size={11}
                            strokeWidth={2}
                            className={styles.appSuggestMetaIcon}
                            aria-hidden
                          />
                          {displayCategoryName(cat, t)}
                        </span>
                      )
                    )}
                  </button>
                );
              })}
              </div>,
              document.body,
            )}
        </span>
      ) : (
        <button
          type="button"
          className={styles.appAddBtn}
          onClick={() => setAdding(true)}
        >
          <Plus size={11} strokeWidth={2} />
          {t("categories.appList.add")}
        </button>
      )}
    </div>
  );
}

interface MenuRect {
  top: number;
  left: number;
  width: number;
}

export function AssignDropdown({
  categories,
  currentCategoryId,
  onPick,
  allowClear = false,
}: {
  categories: Category[];
  /** 当前已选中的 category id；用于在 trigger 上显示当前分类名 + 颜色 */
  currentCategoryId?: string | null;
  /** allowClear=true 时给 null 走「取消分类」语义 */
  onPick: (categoryId: string | null) => void | Promise<void>;
  /** 是否在下拉里加一行「取消分类」（仅在已分类时有意义）*/
  allowClear?: boolean;
}) {
  const { t } = useTranslation();
  const current =
    currentCategoryId != null
      ? categories.find((c) => c.id === currentCategoryId) ?? null
      : null;
  const [open, setOpen] = useState(false);
  const [menuRect, setMenuRect] = useState<MenuRect | null>(null);
  const btnRef = useRef<HTMLButtonElement>(null);
  const menuRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!open) return;
    const onDown = (e: MouseEvent) => {
      const target = e.target as Node;
      if (btnRef.current?.contains(target)) return;
      if (menuRef.current?.contains(target)) return;
      setOpen(false);
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") setOpen(false);
    };
    const onScroll = () => setOpen(false);
    document.addEventListener("mousedown", onDown);
    document.addEventListener("keydown", onKey);
    window.addEventListener("scroll", onScroll, true);
    window.addEventListener("resize", onScroll);
    return () => {
      document.removeEventListener("mousedown", onDown);
      document.removeEventListener("keydown", onKey);
      window.removeEventListener("scroll", onScroll, true);
      window.removeEventListener("resize", onScroll);
    };
  }, [open]);

  const handleToggle = () => {
    if (open) {
      setOpen(false);
      return;
    }
    if (!btnRef.current) return;
    const rect = btnRef.current.getBoundingClientRect();
    const estimatedMenuHeight = categories.length * 28 + 8 + 4;
    const spaceBelow = window.innerHeight - rect.bottom;
    const spaceAbove = rect.top;
    const flipUp = spaceBelow < estimatedMenuHeight && spaceAbove > spaceBelow;

    setMenuRect({
      top: flipUp ? rect.top - estimatedMenuHeight - 4 : rect.bottom + 4,
      left: rect.left,
      width: rect.width,
    });
    setOpen(true);
  };

  // trigger 按 current 是否存在切换显示形态：
  //   未分类 → "+ 指派"
  //   已分类 → "<分类 icon> <分类名>"（icon 染分类色，跟 chip filter 一族对齐）
  const triggerStyle = current
    ? ({ "--cat-color": current.color } as CSSProperties)
    : undefined;
  const CurrentIcon = current ? resolveCategoryIcon(current.icon) : null;

  return (
    <div className={styles.assignWrap}>
      <button
        ref={btnRef}
        type="button"
        className={`${styles.assignBtn} ${open ? styles.assignBtnOpen : ""} ${
          current ? styles.assignBtnPicked : ""
        }`}
        style={triggerStyle}
        onClick={handleToggle}
      >
        {current && CurrentIcon ? (
          <>
            <CurrentIcon
              size={12}
              strokeWidth={2}
              className={styles.assignOptionIcon}
              aria-hidden
            />
            <span className={styles.assignBtnLabel}>{displayCategoryName(current, t)}</span>
          </>
        ) : (
          <>
            <Plus size={11} strokeWidth={2.25} />
            {t("categories.assign.label")}
          </>
        )}
        <ChevronDown size={11} strokeWidth={2.25} className={styles.chev} />
      </button>
      {open && menuRect && (
        <div
          ref={menuRef}
          className={styles.assignMenu}
          role="menu"
          style={{
            top: menuRect.top,
            left: menuRect.left,
            minWidth: menuRect.width,
          }}
        >
          {allowClear && current && (
            <button
              type="button"
              role="menuitem"
              className={`${styles.assignOption} ${styles.assignOptionClear}`}
              onClick={() => {
                setOpen(false);
                void onPick(null);
              }}
            >
              <span className={styles.assignOptionLabel}>
                {t("categories.assign.clear")}
              </span>
            </button>
          )}
          {categories.map((c) => {
            const style = { "--cat-color": c.color } as CSSProperties;
            const isCurrent = current?.id === c.id;
            const Icon = resolveCategoryIcon(c.icon);
            return (
              <button
                key={c.id}
                type="button"
                role="menuitem"
                className={`${styles.assignOption} ${
                  isCurrent ? styles.assignOptionCurrent : ""
                }`}
                style={style}
                onClick={() => {
                  setOpen(false);
                  void onPick(c.id);
                }}
              >
                <Icon
                  size={12}
                  strokeWidth={2}
                  className={styles.assignOptionIcon}
                  aria-hidden
                />
                <span className={styles.assignOptionLabel}>{displayCategoryName(c, t)}</span>
              </button>
            );
          })}
        </div>
      )}
    </div>
  );
}

