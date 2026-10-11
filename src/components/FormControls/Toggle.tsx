import { useContext } from "react";
import { RowLabelContext } from "../FormLayout/rowLabelContext";
import styles from "./Toggle.module.css";

interface ToggleProps {
  checked: boolean;
  onChange: (next: boolean) => void;
  ariaLabel?: string;
  disabled?: boolean;
}

export function Toggle({ checked, onChange, ariaLabel, disabled = false }: ToggleProps) {
  // Row 通过 context 下传 label id；调用方显式给 ariaLabel 时以 ariaLabel 优先
  const rowLabelId = useContext(RowLabelContext);
  return (
    <button
      type="button"
      role="switch"
      aria-checked={checked}
      aria-label={ariaLabel}
      aria-labelledby={ariaLabel ? undefined : rowLabelId}
      disabled={disabled}
      className={`${styles.toggle} ${checked ? styles.on : ""}`}
      onClick={() => onChange(!checked)}
    >
      <span className={styles.thumb} />
    </button>
  );
}
