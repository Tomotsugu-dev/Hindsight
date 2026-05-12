// Demo-only override: Monthly tab 不显示 AI 总结模式切换，直接渲染 Quick 模板。
import { QuickSummaryView } from "@app/pages/AISummary/tabs/QuickSummaryView";

export default function MonthlyTab() {
  return <QuickSummaryView scope="month" />;
}
