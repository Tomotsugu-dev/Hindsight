import { useTranslation } from "react-i18next";
import { EyeOff } from "lucide-react";
import { resolveCategoryIcon } from "../../config/categoryIcons";
import { useCategories } from "../../state/categories";
import styles from "./CategoryChipMultiSelect.module.css";

interface Props {
  /** AI 能看到的分类 id */
  selectedIds: string[];
  /** Leave it out to show the chips read-only. */
  onChange?: (next: string[]) => void;
}

/** 「不计入统计」和「隐私」AI 永远看不到，不给用户选，免得以为勾上就能看到。 */
const NEVER_SHOWN = new Set(["hidden", "private"]);

/** 分类小方块：彩色带图标 = AI 能看到，灰色闭眼 = 看不到。点一下切换。 */
export function CategoryChipMultiSelect({ selectedIds, onChange }: Props) {
  const { t } = useTranslation();
  // 用共享的分类，不自己请求：标签和外面的卡片同一次渲染出来，卡片高度不会事后再变
  const { categories, loading } = useCategories();

  const toggle = (id: string) => {
    if (!onChange) return;
    if (selectedIds.includes(id)) {
      onChange(selectedIds.filter((x) => x !== id));
    } else {
      onChange([...selectedIds, id]);
    }
  };

  if (loading && categories.length === 0) {
    return <div className={styles.loading}>{t("components.categoryChipMultiSelect.loading")}</div>;
  }

  const choices = categories.filter((c) => !c.builtin && !NEVER_SHOWN.has(c.id));
  if (choices.length === 0) {
    return <div className={styles.empty}>{t("components.categoryChipMultiSelect.empty")}</div>;
  }

  // Read-only chips also show the categories the AI never sees, not picked, at the end.
  // Nobody can click them here, so they do not look like something to turn on.
  if (!onChange) {
    const neverShown = categories.filter((c) => c.builtin || NEVER_SHOWN.has(c.id));
    return (
      <div className={styles.chips}>
        {[...choices, ...neverShown].map((c) => {
          const seen = selectedIds.includes(c.id);
          const Icon = resolveCategoryIcon(c.icon);
          return (
            <span
              key={c.id}
              className={`${styles.chip} ${styles.readOnly} ${seen ? "" : styles.excluded}`}
              style={{ background: c.color, borderColor: c.color }}
            >
              <Icon size={13} strokeWidth={2} aria-hidden />
              <span className={styles.name}>{c.name}</span>
            </span>
          );
        })}
      </div>
    );
  }

  return (
    <div className={styles.chips}>
      {choices.map((c) => {
        const seen = selectedIds.includes(c.id);
        const Icon = seen ? resolveCategoryIcon(c.icon) : EyeOff;
        return (
          <button
            key={c.id}
            type="button"
            className={`${styles.chip} ${seen ? "" : styles.excluded}`}
            style={{
              background: c.color,
              borderColor: c.color,
            }}
            onClick={() => toggle(c.id)}
            aria-pressed={seen}
            title={
              seen
                ? t("components.categoryChipMultiSelect.seenTitle")
                : t("components.categoryChipMultiSelect.unseenTitle")
            }
          >
            <Icon size={13} strokeWidth={2} />
            <span className={styles.name}>{c.name}</span>
          </button>
        );
      })}
    </div>
  );
}
