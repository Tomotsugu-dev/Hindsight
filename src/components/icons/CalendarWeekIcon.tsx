import { forwardRef } from "react";
import type { LucideIcon, LucideProps } from "lucide-react";

/**
 * Google Material Symbols `calendar_view_week`（日历里 4 根竖条，周视图意象）。
 * 跟 [`CalendarTodayIcon`] / [`CalendarMonthIcon`] 同款 filled 风格，
 * nav 三件套形成 single-day → week-columns → month-grid 的视觉递进。
 */
export const CalendarWeekIcon: LucideIcon = forwardRef<SVGSVGElement, LucideProps>(
  function CalendarWeekIcon({ size = 24, color = "currentColor", className, ...rest }, ref) {
    return (
      <svg
        ref={ref}
        xmlns="http://www.w3.org/2000/svg"
        width={size}
        height={size}
        viewBox="0 -960 960 960"
        fill={color}
        className={className}
        {...rest}
      >
        <path d="M160-160q-33 0-56.5-23.5T80-240v-480q0-33 23.5-56.5T160-800h640q33 0 56.5 23.5T880-720v480q0 33-23.5 56.5T800-160H160Zm360-80h100v-480H520v480Zm-180 0h100v-480H340v480Zm-180 0h100v-480H160v480Zm540 0h100v-480H700v480Z" />
      </svg>
    );
  },
) as unknown as LucideIcon;
