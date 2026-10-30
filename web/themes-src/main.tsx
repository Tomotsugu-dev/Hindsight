import React from "react";
import ReactDOM from "react-dom/client";
// 顶栏、页脚和配色和官网其它页共用
import "../site.css";
import { ThemesPage } from "./ThemesPage";

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <ThemesPage />
  </React.StrictMode>,
);
