import { useEffect, useRef, useState } from "react";
import { Pencil, Trash2 } from "lucide-react";
import { useCategories } from "../../state/categories";
import type { Category } from "../../api/hindsight";
import { AppList, ColorPalette, CreateCategory, UnclassifiedSection } from "./parts";
import styles from "./Categories.module.css";

export default function CategoriesPage() {
  const { categories, loading } = useCategories();

  return (
    <div className={styles.page}>
      <header className={styles.header}>
        <h1 className={styles.title}>应用分类</h1>
        <p className={styles.meta}>
          不同进程归属到不同的活动类别。颜色、名称、绑定应用都可自定义。
        </p>
      </header>

      <section className={styles.card}>
        {loading && categories.length === 0 ? (
          <div className={styles.empty}>加载中…</div>
        ) : (
          <>
            {categories.map((c) => (
              <CategoryRow key={c.id} category={c} />
            ))}
            <div className={styles.createRow}>
              <CreateCategory />
            </div>
          </>
        )}
      </section>

      <header className={styles.header} style={{ marginTop: 8 }}>
        <h2 className={styles.title} style={{ fontSize: 18 }}>
          未归类
        </h2>
        <p className={styles.meta}>
          近 7 天采集到、还没归类的应用。指派到任意分类立即生效。
        </p>
      </header>

      <section className={styles.card}>
        <UnclassifiedSection />
      </section>
    </div>
  );
}

function CategoryRow({ category }: { category: Category }) {
  const { update, remove } = useCategories();
  const [editingName, setEditingName] = useState(false);
  const [draftName, setDraftName] = useState(category.name);
  const [paletteOpen, setPaletteOpen] = useState(false);
  const inputRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    if (editingName) {
      inputRef.current?.focus();
      inputRef.current?.select();
    }
  }, [editingName]);

  useEffect(() => {
    if (!editingName) setDraftName(category.name);
  }, [category.name, editingName]);

  const commitName = async () => {
    const trimmed = draftName.trim();
    if (trimmed && trimmed !== category.name) {
      await update(category.id, { name: trimmed });
    } else {
      setDraftName(category.name);
    }
    setEditingName(false);
  };

  const cancelName = () => {
    setDraftName(category.name);
    setEditingName(false);
  };

  const pickColor = async (color: string) => {
    setPaletteOpen(false);
    if (color !== category.color) {
      await update(category.id, { color });
    }
  };

  const onDelete = async () => {
    if (!window.confirm(`确认删除分类「${category.name}」？其下应用会回到「其他」。`)) {
      return;
    }
    await remove(category.id);
  };

  return (
    <div className={styles.catRow}>
      <div className={styles.catHead}>
        <div className={styles.popoverWrap}>
          <button
            type="button"
            className={styles.colorChipBtn}
            style={{ background: category.color }}
            onClick={() => setPaletteOpen((v) => !v)}
            aria-label="改颜色"
          />
          {paletteOpen && (
            <ColorPalette
              current={category.color}
              onPick={pickColor}
              onDismiss={() => setPaletteOpen(false)}
            />
          )}
        </div>

        {editingName ? (
          <input
            ref={inputRef}
            className={styles.catNameInput}
            value={draftName}
            maxLength={16}
            onChange={(e) => setDraftName(e.target.value)}
            onBlur={commitName}
            onKeyDown={(e) => {
              if (e.key === "Enter") commitName();
              if (e.key === "Escape") cancelName();
            }}
          />
        ) : (
          <span
            className={styles.catName}
            onDoubleClick={() => setEditingName(true)}
            title="双击改名"
          >
            {category.name}
          </span>
        )}

        <div className={styles.catActions}>
          {!editingName && (
            <button
              type="button"
              className={styles.iconBtn}
              onClick={() => setEditingName(true)}
              aria-label="改名"
              title="改名"
            >
              <Pencil size={12} strokeWidth={1.85} />
            </button>
          )}
          <button
            type="button"
            className={`${styles.iconBtn} ${styles.iconBtnDanger}`}
            onClick={onDelete}
            aria-label="删除分类"
            title="删除分类"
          >
            <Trash2 size={12} strokeWidth={1.85} />
          </button>
        </div>
      </div>

      <AppList category={category} indent />
    </div>
  );
}
