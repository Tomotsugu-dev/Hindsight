import { forwardRef } from "react";
import type { LucideIcon, LucideProps } from "lucide-react";

/**
 * Tabler `settings-ai`——齿轮里嵌 "AI" 字样的 outline 风格。
 * 跟 lucide outline 描边风格一致。页面标题行进 AI 设置的按钮用它，跟进「设置」的普通齿轮区分开。
 *
 * strokeWidth 通过 LucideProps 传入，跟其它 lucide icon 一样。
 */
export const AiSettingsIcon: LucideIcon = forwardRef<SVGSVGElement, LucideProps>(
  function AiSettingsIcon(
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
        <path stroke="none" d="M0 0h24v24H0z" fill="none" />
        <path d="M10.325 4.317c.426 -1.756 2.924 -1.756 3.35 0a1.724 1.724 0 0 0 2.573 1.066c1.543 -.94 3.31 .826 2.37 2.37a1.724 1.724 0 0 0 1.065 2.572c1.756 .426 1.756 2.924 0 3.35a1.724 1.724 0 0 0 -1.066 2.573c.94 1.543 -.826 3.31 -2.37 2.37a1.724 1.724 0 0 0 -2.572 1.065c-.426 1.756 -2.924 1.756 -3.35 0a1.724 1.724 0 0 0 -2.573 -1.066c-1.543 .94 -3.31 -.826 -2.37 -2.37a1.724 1.724 0 0 0 -1.065 -2.572c-1.756 -.426 -1.756 -2.924 0 -3.35a1.724 1.724 0 0 0 1.066 -2.573c-.94 -1.543 .826 -3.31 2.37 -2.37c1 .608 2.296 .07 2.572 -1.065" />
        <path d="M9 14v-2.5a1.5 1.5 0 0 1 3 0v2.5" />
        <path d="M9 13h3" />
        <path d="M15 10v4" />
      </svg>
    );
  },
);
