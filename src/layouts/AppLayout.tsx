import { useState } from "react";
import { Outlet } from "react-router-dom";
import { Sidebar } from "../components/Sidebar/Sidebar";
import { WindowControls } from "../components/WindowControls/WindowControls";
import styles from "./AppLayout.module.css";

export function AppLayout() {
  /** 鼠标进入侧栏时展开，离开时收成图标条 */
  const [expanded, setExpanded] = useState(true);

  return (
    <div className={`${styles.shell} ${expanded ? styles.expanded : ""}`}>
      {/* 顶部隐形拖动带 */}
      <div className={styles.dragStrip} data-tauri-drag-region />

      {/* 主内容区 — left 跟随侧栏宽度，避免被遮 */}
      <main className={styles.content}>
        <Outlet />
      </main>

      {/* 侧栏宿主：rail 56px ↔ full 232px */}
      <div
        className={styles.sidebarHost}
        onMouseEnter={() => setExpanded(true)}
        onMouseLeave={() => setExpanded(false)}
      >
        <Sidebar />
      </div>

      <WindowControls />
    </div>
  );
}
