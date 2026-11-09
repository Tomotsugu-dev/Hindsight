import { useCallback, type ButtonHTMLAttributes, type Ref } from "react";
import { Loader2, type LucideIcon } from "lucide-react";
import { useMouseGlow } from "../../hooks/useMouseGlow";
import styles from "./Button.module.css";

/** How the button is drawn. */
export type ButtonVariant = "solid" | "soft" | "outline" | "ghost";

/**
 * Which color the variant uses: the accent, gray, amber in a warning, or red for actions that
 * delete or remove.
 */
export type ButtonTone = "accent" | "neutral" | "warning" | "danger";

/** The three control heights in tokens.css. */
export type ControlSize = "sm" | "md" | "lg";

/** Buttons also come in xs, for small icons in dense rows. */
export type ButtonSize = "xs" | ControlSize;

export interface ButtonProps extends ButtonHTMLAttributes<HTMLButtonElement> {
  variant?: ButtonVariant;
  tone?: ButtonTone;
  size?: ButtonSize;
  /**
   * Gray until the mouse is on it, then the tone shows, such as a row's delete that turns red.
   * On by default for gray and red ghost buttons. Has no effect on solid buttons.
   */
  quiet?: boolean;
  /** Shown before the text */
  icon?: LucideIcon;
  /** Shows a spinner in place of the icon, and the button cannot be pressed */
  loading?: boolean;
  /** Lights up the border as the mouse comes near, like the arrows that change the period */
  glow?: boolean;
  ref?: Ref<HTMLButtonElement>;
}

/**
 * A button for an action, such as Save or Uninstall.
 *
 * It carries `data-part="button"`, `data-variant`, `data-tone`, `data-size` and `data-quiet`, so
 * a theme can style it. `className` is for layout only, such as a margin; the look comes from the props.
 */
export function Button({
  variant = "soft",
  tone = "neutral",
  size = "md",
  quiet = variant === "ghost" && tone !== "accent",
  icon: Icon,
  loading = false,
  glow = false,
  type = "button",
  className,
  disabled,
  children,
  ref,
  ...rest
}: ButtonProps) {
  // Hooks cannot be called conditionally, so every button has the ref; only a glow button uses it
  const { ref: glowRef } = useMouseGlow<HTMLButtonElement>();
  // The glow and the caller may both need the element
  const setRef = useCallback(
    (el: HTMLButtonElement | null) => {
      if (glow) glowRef.current = el;
      if (typeof ref === "function") ref(el);
      else if (ref) ref.current = el;
    },
    [glow, glowRef, ref],
  );
  const classes = [styles.button, glow ? "glow" : null, className].filter(Boolean).join(" ");
  return (
    <button
      {...rest}
      ref={setRef}
      type={type}
      className={classes}
      data-part="button"
      data-variant={variant}
      data-tone={tone}
      data-size={size}
      data-quiet={quiet || undefined}
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
