import { useEffect, useRef, useState, type CSSProperties } from "react";
import { Check, Pencil, Plus, Trash2, X } from "lucide-react";
import { useCategories } from "../../state/categories";
import type { Category } from "../../api/hindsight";
import { ConfirmDialog } from "../../components/ConfirmDialog/ConfirmDialog";
import { AppearancePicker } from "../../components/AppearancePicker/AppearancePicker";
import { resolveCategoryIcon } from "../../config/categoryIcons";
import { AppList, DEFAULT_PALETTE, UnclassifiedSection } from "./parts";
import { PairingSection } from "./PairingSection";
import styles from "./Categories.module.css";

const DEFAULT_NEW_ICON = "Tag";

export default function CategoriesPage() {
  const { categories, loading, create } = useCategories();
  const [creating, setCreating] = useState(false);

  const handleCreated = async (input: { name: string; color: string; icon: string }) => {
    await create(input);
    setCreating(false);
  };

  return (
    <div className={styles.page}>
      <header className={styles.header}>
        <div className={styles.headerText}>
          <h1 className={styles.title}>应用分类</h1>
          <p className={styles.meta}>
            不同进程归属到不同的活动类别。颜色、图标、名称、绑定应用都可自定义。
          </p>
        </div>
        <button
          type="button"
          className={styles.createBtn}
          onClick={() => setCreating(true)}
          disabled={creating}
        >
          <Plus size={14} strokeWidth={2.25} />
          新建分类
        </button>
      </header>

      <section className={styles.card}>
        {creating && (
          <CreatingRow
            onCommit={handleCreated}
            onCancel={() => setCreating(false)}
          />
        )}
        {loading && categories.length === 0 ? (
          <div className={styles.empty}>加载中…</div>
        ) : (
          categories.map((c) => <CategoryRow key={c.id} category={c} />)
        )}
      </section>

      <header className={styles.header} style={{ marginTop: 8 }}>
        <div className={styles.headerText}>
          <h2 className={styles.title} style={{ fontSize: 18 }}>
            跨设备配对
          </h2>
          <p className={styles.meta}>
            xcap 在 mac / win 上对同一应用返回不同进程名（mac="Code" / win="Visual Studio Code"）。
            把不同设备上的同一应用拖到一行里就合并成一组，分类一次跨平台联动。
          </p>
        </div>
      </header>

      <section className={styles.card}>
        <PairingSection />
      </section>

      <header className={styles.header} style={{ marginTop: 8 }}>
        <div className={styles.headerText}>
          <h2 className={styles.title} style={{ fontSize: 18 }}>
            未归类
          </h2>
          <p className={styles.meta}>
            近 7 天采集到、还没归类的应用。指派到任意分类立即生效（如果配对过会联动整组）。
          </p>
        </div>
      </header>

      <section
        className={styles.card}
        style={{
          background: "#fbfbfd",
          borderRadius: 14,
          border: "1px solid rgba(0,0,0,0.06)",
        }}
      >
        <UnclassifiedSection />
      </section>
    </div>
  );
}

function CategoryRow({ category }: { category: Category }) {
  const { update, remove } = useCategories();
  const [editingName, setEditingName] = useState(false);
  const [draftName, setDraftName] = useState(category.name);
  const [pickerOpen, setPickerOpen] = useState(false);
  const [confirmOpen, setConfirmOpen] = useState(false);
  const inputRef = useRef<HTMLInputElement>(null);
  const Icon = resolveCategoryIcon(category.icon);

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

  const onConfirmDelete = async () => {
    setConfirmOpen(false);
    await remove(category.id);
  };

  const styleVar = { "--cat-color": category.color } as CSSProperties;

  return (
    <div className={styles.catRow} style={styleVar}>
      <div className={styles.catIconWrap}>
        <button
          type="button"
          className={styles.catIconBtn}
          onClick={() => setPickerOpen((v) => !v)}
          aria-label="改外观"
          title="改颜色和图标"
        >
          <Icon size={28} strokeWidth={1.85} />
        </button>
        {pickerOpen && (
          <AppearancePicker
            color={category.color}
            icon={category.icon}
            onColorChange={(c) => update(category.id, { color: c })}
            onIconChange={(i) => {
              update(category.id, { icon: i });
              setPickerOpen(false);
            }}
            onDismiss={() => setPickerOpen(false)}
          />
        )}
      </div>

      <div className={styles.catBody}>
        <div className={styles.catNameRow}>
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
        </div>

        <div className={styles.appListClip}>
          <AppList category={category} />
        </div>
      </div>

      <div className={styles.catActions}>
        <button
          type="button"
          className={styles.actionBtn}
          onClick={() => setEditingName(true)}
          aria-label="改名"
          title="改名"
        >
          <Pencil size={14} strokeWidth={1.85} />
        </button>
        <button
          type="button"
          className={`${styles.actionBtn} ${styles.actionBtnDanger}`}
          onClick={() => setConfirmOpen(true)}
          aria-label="删除分类"
          title="删除分类"
        >
          <Trash2 size={14} strokeWidth={1.85} />
        </button>
      </div>

      <ConfirmDialog
        open={confirmOpen}
        title="删除分类"
        message={`确认删除分类「${category.name}」？其下应用会回到「其他」。`}
        confirmLabel="删除"
        variant="danger"
        onConfirm={onConfirmDelete}
        onCancel={() => setConfirmOpen(false)}
      />
    </div>
  );
}

function CreatingRow({
  onCommit,
  onCancel,
}: {
  onCommit: (input: { name: string; color: string; icon: string }) => void | Promise<void>;
  onCancel: () => void;
}) {
  const [name, setName] = useState("");
  const [color, setColor] = useState(DEFAULT_PALETTE[0]);
  const [icon, setIcon] = useState(DEFAULT_NEW_ICON);
  const [pickerOpen, setPickerOpen] = useState(false);
  const inputRef = useRef<HTMLInputElement>(null);
  const Icon = resolveCategoryIcon(icon);

  useEffect(() => {
    inputRef.current?.focus();
  }, []);

  const commit = async () => {
    const trimmed = name.trim();
    if (!trimmed) {
      onCancel();
      return;
    }
    await onCommit({ name: trimmed, color, icon });
  };

  const styleVar = { "--cat-color": color } as CSSProperties;

  return (
    <div className={styles.catRow} style={styleVar}>
      <div className={styles.catIconWrap}>
        <button
          type="button"
          className={styles.catIconBtn}
          onClick={() => setPickerOpen((v) => !v)}
          aria-label="选颜色和图标"
          title="选颜色和图标"
        >
          <Icon size={28} strokeWidth={1.85} />
        </button>
        {pickerOpen && (
          <AppearancePicker
            color={color}
            icon={icon}
            onColorChange={setColor}
            onIconChange={(i) => {
              setIcon(i);
              setPickerOpen(false);
            }}
            onDismiss={() => setPickerOpen(false)}
          />
        )}
      </div>

      <div className={styles.catBody}>
        <div className={styles.catNameRow}>
          <input
            ref={inputRef}
            className={styles.catNameInput}
            placeholder="分类名"
            value={name}
            maxLength={16}
            onChange={(e) => setName(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter") commit();
              if (e.key === "Escape") onCancel();
            }}
          />
        </div>
      </div>

      <div className={styles.catActions}>
        <button
          type="button"
          className={`${styles.actionBtn}`}
          onMouseDown={(e) => e.preventDefault()}
          onClick={commit}
          aria-label="确认"
          title="确认"
        >
          <Check size={14} strokeWidth={2.25} />
        </button>
        <button
          type="button"
          className={`${styles.actionBtn} ${styles.actionBtnDanger}`}
          onMouseDown={(e) => e.preventDefault()}
          onClick={onCancel}
          aria-label="取消"
          title="取消"
        >
          <X size={14} strokeWidth={2.25} />
        </button>
      </div>
    </div>
  );
}
