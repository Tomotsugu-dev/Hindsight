import { Outlet } from "react-router-dom";
import { Sidebar } from "../components/Sidebar/Sidebar";
import { WindowControls } from "../components/WindowControls/WindowControls";
import styles from "./AppLayout.module.css";

export function AppLayout() {
  return (
    <div className={styles.shell}>
      <div className={styles.dragStrip} data-tauri-drag-region />

      <main className={styles.content}>
        <Outlet />
      </main>

      <div className={styles.sidebarHost}>
        <Sidebar />
      </div>

      <WindowControls />
    </div>
  );
}
