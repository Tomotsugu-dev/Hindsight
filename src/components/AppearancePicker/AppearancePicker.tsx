import { useEffect, useRef, type CSSProperties } from "react";
import { CATEGORY_ICONS, ICON_NAMES } from "../../config/categoryIcons";
import styles from "./AppearancePicker.module.css";

const PALETTE = [
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

interface AppearancePickerProps {
  color: string;
  icon: string;
  onColorChange: (color: string) => void;
  onIconChange: (icon: string) => void;
  onDismiss: () => void;
}

export function AppearancePicker({
  color,
  icon,
  onColorChange,
  onIconChange,
  onDismiss,
}: AppearancePickerProps) {
  const ref = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const onDown = (e: MouseEvent) => {
      if (!ref.current) return;
      if (!ref.current.contains(e.target as Node)) onDismiss();
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onDismiss();
    };
    document.addEventListener("mousedown", onDown);
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("mousedown", onDown);
      document.removeEventListener("keydown", onKey);
    };
  }, [onDismiss]);

  const styleVar = { "--cat-color": color } as CSSProperties;

  return (
    <div ref={ref} className={styles.popover} style={styleVar}>
      <div className={styles.section}>
        <span className={styles.label}>颜色</span>
        <div className={styles.colorRow}>
          {PALETTE.map((c) => (
            <button
              key={c}
              type="button"
              className={`${styles.swatch} ${
                c.toLowerCase() === color.toLowerCase() ? styles.swatchActive : ""
              }`}
              style={{ background: c }}
              onClick={() => onColorChange(c)}
              aria-label={c}
            />
          ))}
        </div>
      </div>

      <div className={styles.section}>
        <span className={styles.label}>图标</span>
        <div className={styles.iconGrid}>
          {ICON_NAMES.map((name) => {
            const Icon = CATEGORY_ICONS[name];
            const active = name === icon;
            return (
              <button
                key={name}
                type="button"
                className={`${styles.iconBtn} ${active ? styles.iconBtnActive : ""}`}
                onClick={() => onIconChange(name)}
                aria-label={name}
                title={name}
              >
                <Icon size={16} strokeWidth={1.85} />
              </button>
            );
          })}
        </div>
      </div>
    </div>
  );
}
