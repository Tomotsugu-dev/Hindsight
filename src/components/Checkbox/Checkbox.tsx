import type { InputHTMLAttributes, ReactNode, Ref } from "react";
import styles from "./Checkbox.module.css";

export interface CheckboxProps extends Omit<InputHTMLAttributes<HTMLInputElement>, "type"> {
  /** The text next to the box. Without it, give the box an aria-label */
  children?: ReactNode;
  ref?: Ref<HTMLInputElement>;
}

/**
 * A checkbox with its text. `className` goes on the label, for layout; the other props go on
 * the input. The label carries `data-part="checkbox"` for themes.
 */
export function Checkbox({ children, className, ref, ...rest }: CheckboxProps) {
  return (
    <label
      className={className ? `${styles.checkbox} ${className}` : styles.checkbox}
      data-part="checkbox"
    >
      <input {...rest} ref={ref} type="checkbox" className={styles.box} />
      {children !== undefined ? <span>{children}</span> : null}
    </label>
  );
}
