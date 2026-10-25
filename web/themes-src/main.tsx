import React from "react";
import ReactDOM from "react-dom/client";
// 官网首页的样式：导航栏和配色跟首页一样
import "../styles.css";
import { ThemesPage } from "./ThemesPage";

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <ThemesPage />
  </React.StrictMode>,
);
