import { forwardRef } from "react";
import type { LucideIcon, LucideProps } from "lucide-react";

/**
 * 柱状图加一道斜杠：「不计入统计」。lucide 没有这个图标，用它的 `chart-no-axes-column`
 * 加上 `-off` 系列同款的斜杠拼成，描边风格和其它 lucide 图标一致。
 */
export const ChartOffIcon: LucideIcon = forwardRef<SVGSVGElement, LucideProps>(
  function ChartOffIcon(
    { size = 24, color = "currentColor", strokeWidth = 2, className, ...rest },
    ref,
  ) {
    return (
      <svg
        ref={ref}
        xmlns="http://www.w3.org/2000/svg"
        width={size}
        height={size}
        viewBox="0 0 24 24"
        fill="none"
        stroke={color}
        strokeWidth={strokeWidth}
        strokeLinecap="round"
        strokeLinejoin="round"
        className={className}
        {...rest}
      >
        <path d="M5 21v-6" />
        <path d="M12 21V3" />
        <path d="M19 21V9" />
        <path d="m2 2 20 20" />
      </svg>
    );
  },
);
