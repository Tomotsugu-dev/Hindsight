import { forwardRef } from "react";
import type { LucideIcon, LucideProps } from "lucide-react";

/**
 * Google Material Symbols `calendar_today` (filled 风格)。
 * 包成 LucideIcon 形状（forwardRef + 接 LucideProps），可以直接塞进
 * 任何吃 LucideIcon 的地方（NavItem / Section header / 等等）。
 *
 * 跟 lucide 自家 outline 描边风格不同——这是 Material Symbols 的填充风格 icon，
 * 视觉权重略重，用来给"今日总览"做强标识。`color="currentColor"` 让它跟周围
 * 文字色一致，跟 lucide icon 行为对齐。
 */
export const CalendarTodayIcon: LucideIcon = forwardRef<SVGSVGElement, LucideProps>(
  function CalendarTodayIcon({ size = 24, color = "currentColor", className, ...rest }, ref) {
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
        <path d="M200-80q-33 0-56.5-23.5T120-160v-560q0-33 23.5-56.5T200-800h40v-80h80v80h320v-80h80v80h40q33 0 56.5 23.5T840-720v560q0 33-23.5 56.5T760-80H200Zm0-80h560v-400H200v400Zm0-480h560v-80H200v80Zm0 0v-80 80Z" />
      </svg>
    );
  },
) as unknown as LucideIcon;
