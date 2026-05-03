import { useEffect, useRef, useState, type CSSProperties } from "react";
import { ChevronDown, Plus, X } from "lucide-react";
import { useCategories } from "../../state/categories";
import { api, type Category, type UnclassifiedApp } from "../../api/hindsight";
import { AppIcon } from "../../components/AppIcon/AppIcon";
import styles from "./Categories.module.css";

export const DEFAULT_PALETTE = [
  "#a78bfa",
  "#60a5fa",
  "#34d399",
  "#fbbf24",
  "#fb7185",
  "#94a3b8",
  "#f97316",
  "#3b82f6",
  "#10b981",
  "#d946ef",
  "#06b6d4",
  "#facc15",
];

export function AppList({ category }: { category: Category }) {
  const { unassignApp, assignApp } = useCategories();
  const [adding, setAdding] = useState(false);
  const [draft, setDraft] = useState("");
  const inputRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    if (adding) inputRef.current?.focus();
  }, [adding]);

  const commit = async () => {
    const trimmed = draft.trim();
    if (trimmed) {
      await assignApp(trimmed, category.id);
    }
    setDraft("");
    setAdding(false);
  };

  const cancel = () => {
    setDraft("");
    setAdding(false);
  };

  return (
    <div className={styles.appList}>
      {category.apps.length === 0 && !adding && (
        <span className={styles.empty} style={{ padding: 0 }}>
          （暂无绑定应用）
        </span>
      )}
      {category.apps.map((app) => (
        <span key={app} className={styles.appChip}>
          <AppIcon processName={app} fallbackColor={category.color} size={14} />
          {app}
          <button
            type="button"
            className={styles.appChipRemove}
            onClick={() => unassignApp(app)}
            aria-label={`移除 ${app}`}
            title="移除"
          >
            <X size={10} strokeWidth={2.25} />
          </button>
        </span>
      ))}
      {adding ? (
        <input
          ref={inputRef}
          className={styles.appAddInput}
          placeholder="如 chrome.exe"
          value={draft}
          maxLength={64}
          onChange={(e) => setDraft(e.target.value)}
          onBlur={commit}
          onKeyDown={(e) => {
            if (e.key === "Enter") commit();
            if (e.key === "Escape") cancel();
          }}
        />
      ) : (
        <button
          type="button"
          className={styles.appAddBtn}
          onClick={() => setAdding(true)}
        >
          <Plus size={11} strokeWidth={2} />
          添加应用
        </button>
      )}
    </div>
  );
}

export function UnclassifiedSection() {
  const { categories, assignApp } = useCategories();
  const [items, setItems] = useState<UnclassifiedApp[] | null>(null);

  const reload = async () => {
    try {
      const list = await api.listUnclassifiedApps(7);
      setItems(list);
    } catch {
      setItems([]);
    }
  };

  useEffect(() => {
    reload();
  }, []);

  if (items === null) {
    return <div className={styles.empty}>加载中…</div>;
  }
  if (items.length === 0) {
    return <div className={styles.empty}>所有应用都已归类。</div>;
  }

  return (
    <>
      {items.map((it) => (
        <div key={it.processName} className={styles.unclassRow}>
          <AppIcon processName={it.processName} fallbackColor="#94a3b8" size={18} />
          <span className={styles.unclassName} title={it.processName}>
            {it.processName}
          </span>
          <span className={styles.unclassMeta}>近 7 天 {fmtMin(it.minutes)}</span>
          <AssignDropdown
            categories={categories}
            onPick={async (cid) => {
              await assignApp(it.processName, cid);
              await reload();
            }}
          />
        </div>
      ))}
    </>
  );
}

interface MenuRect {
  top: number;
  left: number;
  width: number;
}

function AssignDropdown({
  categories,
  onPick,
}: {
  categories: Category[];
  onPick: (categoryId: string) => void | Promise<void>;
}) {
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

  return (
    <div className={styles.assignWrap}>
      <button
        ref={btnRef}
        type="button"
        className={`${styles.assignBtn} ${open ? styles.assignBtnOpen : ""}`}
        onClick={handleToggle}
      >
        <Plus size={11} strokeWidth={2.25} />
        指派
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
            width: menuRect.width,
          }}
        >
          {categories.map((c) => {
            const style = { "--cat-color": c.color } as CSSProperties;
            return (
              <button
                key={c.id}
                type="button"
                role="menuitem"
                className={styles.assignOption}
                style={style}
                onClick={() => {
                  setOpen(false);
                  void onPick(c.id);
                }}
              >
                <span className={styles.assignOptionDot} aria-hidden />
                <span className={styles.assignOptionLabel}>{c.name}</span>
              </button>
            );
          })}
        </div>
      )}
    </div>
  );
}

function fmtMin(min: number): string {
  if (min === 0) return "—";
  const h = Math.floor(min / 60);
  const m = min % 60;
  if (h === 0) return `${m} 分`;
  if (m === 0) return `${h} 小时`;
  return `${h} 小时 ${m} 分`;
}
