import { useRef } from "react";
import { Outlet, useLocation } from "react-router-dom";
import { Sidebar } from "../components/Sidebar/Sidebar";
import { WindowControls } from "../components/WindowControls/WindowControls";
import { ErrorBoundary } from "../components/ErrorBoundary/ErrorBoundary";
import { ScrollIndicator } from "../components/ScrollIndicator/ScrollIndicator";
import styles from "./AppLayout.module.css";

export function AppLayout() {
  const location = useLocation();
  const contentRef = useRef<HTMLElement>(null);
  return (
    <div className={styles.shell}>
      <div className={styles.dragStrip} data-tauri-drag-region />

      <main ref={contentRef} className={styles.content}>
        {/* 页面级边界：单页崩溃只换掉内容区，侧栏/窗口 chrome 仍在；
            key=路由让用户切到别的页时边界重挂、自动恢复 */}
        <ErrorBoundary key={location.pathname} scope="page.crash">
          <Outlet />
        </ErrorBoundary>
      </main>
      {/* 2A:窗口右缘的常驻滚动指示条——.content 原生滚动条是藏死的,
          用户此前没有任何"下面还有内容"的信号(自绘原因见 ScrollIndicator) */}
      <ScrollIndicator targetRef={contentRef} className={styles.pageScrollTrack} />

      <div className={styles.sidebarHost}>
        <Sidebar />
      </div>

      <WindowControls />
    </div>
  );
}
