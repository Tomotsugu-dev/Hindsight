import { Plus, X } from "lucide-react";
import { useTranslation } from "react-i18next";
import { TimeField } from "./TimeField";
import styles from "./TimeRangeList.module.css";

export interface TimeRange {
  start: string; // "HH:mm"
  end: string;
}

interface TimeRangeListProps {
  ranges: TimeRange[];
  onChange: (next: TimeRange[]) => void;
}

export function TimeRangeList({ ranges, onChange }: TimeRangeListProps) {
  const { t } = useTranslation();
  const update = (index: number, patch: Partial<TimeRange>) => {
    const next = ranges.map((r, i) => (i === index ? { ...r, ...patch } : r));
    onChange(next);
  };

  const remove = (index: number) => {
    onChange(ranges.filter((_, i) => i !== index));
  };

  const add = () => {
    onChange([...ranges, { start: "09:00", end: "18:00" }]);
  };

  return (
    <div className={styles.list}>
      {ranges.length === 0 ? (
        <div className={styles.empty}>{t("components.timeRangeList.empty")}</div>
      ) : (
        ranges.map((range, i) => (
          <div key={i} className={styles.row}>
            <TimeField
              value={range.start}
              onChange={(v) => update(i, { start: v })}
            />
            <span className={styles.dash}>{t("components.timeRangeList.to")}</span>
            <TimeField
              value={range.end}
              onChange={(v) => update(i, { end: v })}
            />
            <button
              type="button"
              className={styles.remove}
              onClick={() => remove(i)}
              aria-label={t("components.timeRangeList.removeAria")}
            >
              <X size={13} strokeWidth={2} />
            </button>
          </div>
        ))
      )}
      <button type="button" className={styles.add} onClick={add}>
        <Plus size={14} strokeWidth={2} />
        {t("components.timeRangeList.addBtn")}
      </button>
    </div>
  );
}
