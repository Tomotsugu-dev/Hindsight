import { createContext } from "react";

// Row 把它 label <span> 的 id 通过 context 下传，控件（Toggle/Slider）消费它
// 设 aria-labelledby，从而拿到无障碍名。children 是不透明 ReactNode，
// 用 context 比 cloneElement 注入更稳。
export const RowLabelContext = createContext<string | undefined>(undefined);
