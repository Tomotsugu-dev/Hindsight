import type { LucideIcon } from "lucide-react";
import { Button, type ButtonProps } from "./Button";
import styles from "./Button.module.css";

export interface IconButtonProps extends Omit<ButtonProps, "icon" | "children"> {
  icon: LucideIcon;
  /** Read out by screen readers, and shown on hover */
  label: string;
}

/** A square button with only an icon, such as Close or More. It is a Button without text. */
export function IconButton({
  icon,
  label,
  variant = "ghost",
  className,
  title,
  ...rest
}: IconButtonProps) {
  return (
    <Button
      {...rest}
      icon={icon}
      variant={variant}
      aria-label={label}
      title={title ?? label}
      className={className ? `${styles.square} ${className}` : styles.square}
    />
  );
}
