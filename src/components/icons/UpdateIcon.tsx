import { forwardRef } from "react";
import type { LucideIcon, LucideProps } from "lucide-react";

/**
 * Google Material Symbols `update`（圆形箭头 + 时针指向 4 点 5 分）。
 * AboutTab 的「App Updates」section header 用它替代 Sparkles，
 * 跟"刷新版本 / 检查更新"语义对得上。
 */
export const UpdateIcon: LucideIcon = forwardRef<SVGSVGElement, LucideProps>(
  function UpdateIcon({ size = 24, color = "currentColor", className, ...rest }, ref) {
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
        <path d="M480-120q-75 0-140.5-28.5t-114-77q-48.5-48.5-77-114T120-480q0-75 28.5-140.5t77-114q48.5-48.5 114-77T480-840q82 0 155.5 35T760-706v-94h80v240H600v-80h110q-41-56-101-88t-129-32q-117 0-198.5 81.5T200-480q0 117 81.5 198.5T480-200q105 0 183.5-68T756-440h82q-15 137-117.5 228.5T480-120Zm112-192L440-464v-216h80v184l128 128-56 56Z" />
      </svg>
    );
  },
) as unknown as LucideIcon;
