import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { EyeOff } from "lucide-react";
import { api, type Category } from "../../api/hindsight";
import { resolveCategoryIcon } from "../../config/categoryIcons";
import { displayCategoryName } from "../../utils/categoryName";
import styles from "./CategoryChipMultiSelect.module.css";

interface Props {
  selectedIds: string[];
  onChange: (next: string[]) => void;
}

export function CategoryChipMultiSelect({ selectedIds, onChange }: Props) {
  const { t } = useTranslation();
  const [categories, setCategories] = useState<Category[]>([]);
  const [loaded, setLoaded] = useState(false);

  useEffect(() => {
    let cancelled = false;
    api
      .listCategories()
      .then((list) => {
        if (cancelled) return;
        setCategories(list);
        setLoaded(true);
      })
      .catch(() => {
        if (cancelled) return;
        setLoaded(true);
      });
    return () => {
      cancelled = true;
    };
  }, []);

  const toggle = (id: string) => {
    if (selectedIds.includes(id)) {
      onChange(selectedIds.filter((x) => x !== id));
    } else {
      onChange([...selectedIds, id]);
    }
  };

  if (!loaded) {
    return <div className={styles.loading}>{t("components.categoryChipMultiSelect.loading")}</div>;
  }

  if (categories.length === 0) {
    return <div className={styles.empty}>{t("components.categoryChipMultiSelect.empty")}</div>;
  }

  return (
    <div className={styles.chips}>
      {categories.map((c) => {
        const excluded = selectedIds.includes(c.id);
        // 已排除时换成 EyeOff 图标，语义上"AI 看不到这个分类"
        const Icon = excluded ? EyeOff : resolveCategoryIcon(c.icon);
        return (
          <button
            key={c.id}
            type="button"
            className={`${styles.chip} ${excluded ? styles.excluded : ""}`}
            style={{
              background: c.color,
              borderColor: c.color,
            }}
            onClick={() => toggle(c.id)}
            aria-pressed={excluded}
            title={
              excluded
                ? t("components.categoryChipMultiSelect.excludedTitle")
                : t("components.categoryChipMultiSelect.includeTitle")
            }
          >
            <Icon size={13} strokeWidth={2} />
            <span className={styles.name}>{displayCategoryName(c, t)}</span>
          </button>
        );
      })}
    </div>
  );
}
