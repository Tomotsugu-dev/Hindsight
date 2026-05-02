import { FolderOpen } from "lucide-react";
import styles from "./PathField.module.css";

interface PathFieldProps {
  value: string;
  onChange: (next: string) => void;
  onPick?: () => void;
}

export function PathField({ value, onChange, onPick }: PathFieldProps) {
  return (
    <div className={styles.wrap}>
      <input
        className={styles.input}
        value={value}
        onChange={(e) => onChange(e.target.value)}
        spellCheck={false}
      />
      <button type="button" className={styles.pick} onClick={onPick}>
        <FolderOpen size={14} strokeWidth={1.85} />
        选择
      </button>
    </div>
  );
}
