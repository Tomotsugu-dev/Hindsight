/** 新建分类时备选色板。
 *  原来定在 parts.tsx 里跟组件一起 export，会触发 react-refresh/only-export-components
 *  警告（混合组件 + 常量导出会让 HMR 退化）。挪到独立文件后两端 fast refresh 都干净。 */
export const DEFAULT_PALETTE = [
  "#a78bfa",
  "#60a5fa",
  "#34d399",
  "#fbbf24",
  "#fb7185",
  "#94a3b8",
  "#f97316",
  "#3b82f6",
  "#10b981",
  "#d946ef",
  "#06b6d4",
  "#facc15",
];
