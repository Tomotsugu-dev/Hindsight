import type { ButtonHTMLAttributes } from "react";
import { Loader2, type LucideIcon } from "lucide-react";
import styles from "./Button.module.css";

/** How the button is drawn. */
export type ButtonVariant = "solid" | "soft" | "outline" | "ghost";

/** Which color the variant uses: the accent, gray, or red for actions that delete or remove. */
export type ButtonTone = "accent" | "neutral" | "danger";

/** The three control heights in tokens.css. */
export type ControlSize = "sm" | "md" | "lg";

export interface ButtonProps extends ButtonHTMLAttributes<HTMLButtonElement> {
  variant?: ButtonVariant;
  tone?: ButtonTone;
  size?: ControlSize;
  /** Shown before the text */
  icon?: LucideIcon;
  /** Shows a spinner in place of the icon, and the button cannot be pressed */
  loading?: boolean;
}

/**
 * A button for an action, such as Save or Uninstall.
 *
 * It carries `data-part="button"`, `data-variant`, `data-tone` and `data-size`, so a theme can
 * style it. `className` is for layout only, such as a margin; the look comes from the props.
 */
export function Button({
  variant = "soft",
  tone = "neutral",
  size = "md",
  icon: Icon,
  loading = false,
  type = "button",
  className,
  disabled,
  children,
  ...rest
}: ButtonProps) {
  return (
    <button
      {...rest}
      type={type}
      className={className ? `${styles.button} ${className}` : styles.button}
      data-part="button"
      data-variant={variant}
      data-tone={tone}
      data-size={size}
      disabled={disabled || loading}
      aria-busy={loading || undefined}
    >
      {loading ? (
        <Loader2 className={`${styles.icon} ${styles.spinning}`} aria-hidden />
      ) : Icon ? (
        <Icon className={styles.icon} strokeWidth={1.85} aria-hidden />
      ) : null}
      {children}
    </button>
  );
}
