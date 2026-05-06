import { FolderOpen } from "lucide-react";
import { useTranslation } from "react-i18next";
import styles from "./PathField.module.css";

interface PathFieldProps {
  value: string;
  onChange?: (next: string) => void;
  onPick?: () => void;
  /** 输入框是否只读（按钮仍可点） */
  readOnly?: boolean;
  /** 按钮文字，默认走 i18n 中的 components.pathField.pickLabel */
  pickLabel?: string;
}

export function PathField({
  value,
  onChange,
  onPick,
  readOnly,
  pickLabel,
}: PathFieldProps) {
  const { t } = useTranslation();
  const label = pickLabel ?? t("components.pathField.pickLabel");
  return (
    <div className={styles.wrap}>
      <input
        className={styles.input}
        value={value}
        onChange={(e) => onChange?.(e.target.value)}
        spellCheck={false}
        readOnly={readOnly}
      />
      <button type="button" className={styles.pick} onClick={onPick}>
        <FolderOpen size={14} strokeWidth={1.85} />
        {label}
      </button>
    </div>
  );
}
