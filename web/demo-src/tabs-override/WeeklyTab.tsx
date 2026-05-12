// Demo-only override: Weekly tab 不显示 AI 总结模式切换，直接渲染 Quick 模板。
// 通过 web/vite.config.ts 的 alias 把 @app 主 src 里的 WeeklyTab.tsx 替换成本文件。
import { QuickSummaryView } from "@app/pages/AISummary/tabs/QuickSummaryView";

export default function WeeklyTab() {
  return <QuickSummaryView scope="week" />;
}
