import type { ReactNode } from "react";
import { ChevronLeft, ChevronRight } from "lucide-react";
import { Button } from "../Button/Button";
import { IconButton } from "../Button/IconButton";
import styles from "./PeriodNav.module.css";

export interface PeriodNavProps {
  /** The pill's text, such as "Today" or a date */
  label: ReactNode;
  /** The pill's tooltip while it can take you back, such as "Back to today" */
  currentTitle?: string;
  prevLabel: string;
  nextLabel: string;
  /** Names the three buttons as one control for screen readers */
  groupLabel?: string;
  /** The current period is shown, so the pill cannot be pressed */
  isCurrent: boolean;
  canPrev?: boolean;
  canNext: boolean;
  /** None of the three can be pressed, such as while the view slides */
  busy?: boolean;
  onPrev: () => void;
  onNext: () => void;
  onCurrent: () => void;
}

/** ‹ [Today] ›: the arrows step one period back or forward, and the pill goes back to the current one. */
export function PeriodNav({
  label,
  currentTitle,
  prevLabel,
  nextLabel,
  groupLabel,
  isCurrent,
  canPrev = true,
  canNext,
  busy = false,
  onPrev,
  onNext,
  onCurrent,
}: PeriodNavProps) {
  return (
    <div className={styles.nav} role={groupLabel ? "group" : undefined} aria-label={groupLabel}>
      <IconButton
        icon={ChevronLeft}
        label={prevLabel}
        variant="soft"
        size="sm"
        glow
        onClick={onPrev}
        disabled={busy || !canPrev}
      />
      <Button
        variant="soft"
        size="sm"
        glow
        className={styles.pill}
        onClick={onCurrent}
        disabled={busy || isCurrent}
        title={isCurrent ? undefined : currentTitle}
      >
        {label}
      </Button>
      <IconButton
        icon={ChevronRight}
        label={nextLabel}
        variant="soft"
        size="sm"
        glow
        onClick={onNext}
        disabled={busy || !canNext}
      />
    </div>
  );
}
