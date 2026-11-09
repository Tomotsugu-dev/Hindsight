import type { InputHTMLAttributes, Ref } from "react";
import type { LucideIcon } from "lucide-react";
import type { ControlSize } from "../Button/Button";
import styles from "./TextInput.module.css";

export interface TextInputProps extends Omit<InputHTMLAttributes<HTMLInputElement>, "size"> {
  size?: ControlSize;
  /** Shown before the text, such as a search icon */
  icon?: LucideIcon;
  ref?: Ref<HTMLInputElement>;
}

/**
 * A one-line text box. `className` goes on the box, for its width and position; the other
 * props go on the input.
 *
 * The box carries `data-part="input"` and `data-size` for themes. The icon and the input sit
 * side by side in it, so they stay apart at any size.
 */
export function TextInput({
  size = "md",
  icon: Icon,
  type = "text",
  className,
  ref,
  ...rest
}: TextInputProps) {
  return (
    // A label, so a click anywhere in the box focuses the input
    <label
      className={className ? `${styles.box} ${className}` : styles.box}
      data-part="input"
      data-size={size}
    >
      {Icon ? <Icon className={styles.icon} strokeWidth={2} aria-hidden /> : null}
      <input {...rest} ref={ref} type={type} className={styles.input} />
    </label>
  );
}
