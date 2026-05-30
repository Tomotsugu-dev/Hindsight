import { Outlet, useLocation } from "react-router-dom";
import { Sidebar } from "../components/Sidebar/Sidebar";
import { WindowControls } from "../components/WindowControls/WindowControls";
import { ErrorBoundary } from "../components/ErrorBoundary/ErrorBoundary";
import styles from "./AppLayout.module.css";

export function AppLayout() {
  const location = useLocation();
  return (
    <div className={styles.shell}>
      <div className={styles.dragStrip} data-tauri-drag-region />

      <main className={styles.content}>
        {/* 页面级边界：单页崩溃只换掉内容区，侧栏/窗口 chrome 仍在；
            key=路由让用户切到别的页时边界重挂、自动恢复 */}
        <ErrorBoundary key={location.pathname} scope="page.crash">
          <Outlet />
        </ErrorBoundary>
      </main>

      <div className={styles.sidebarHost}>
        <Sidebar />
      </div>

      <WindowControls />
    </div>
  );
}
