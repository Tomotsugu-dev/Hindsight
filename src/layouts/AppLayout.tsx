import { useCallback, useEffect, useRef, useState } from "react";
import { Outlet, useLocation } from "react-router-dom";
import { Sidebar } from "../components/Sidebar/Sidebar";
import { WindowControls } from "../components/WindowControls/WindowControls";
import { ErrorBoundary } from "../components/ErrorBoundary/ErrorBoundary";
import { ScrollIndicator } from "../components/ScrollIndicator/ScrollIndicator";
import BackfillBanner from "../components/BackfillBanner/BackfillBanner";
import { api, type MemoryPendingStats } from "../api/hindsight";
import { logError } from "../lib/logger";
import styles from "./AppLayout.module.css";

/** 全局索引横幅的 stats 兜底刷新间隔:横幅运行态自身 3s 轮询,
 *  这里只负责"没在跑时也能发现新积压"。 */
const PENDING_STATS_REFRESH_MS = 60_000;

export function AppLayout() {
  const location = useLocation();
  const contentRef = useRef<HTMLElement>(null);
  // 未入索引横幅(全局):定时批/常驻批在任何页面跑,这里都可见可停
  const [pendingStats, setPendingStats] = useState<MemoryPendingStats | null>(null);
  const refreshPendingStats = useCallback(() => {
    api
      .memoryPendingStats()
      .then(setPendingStats)
      .catch((e) => logError("layout.pendingStats", e));
  }, []);
  useEffect(() => {
    refreshPendingStats();
    const timer = setInterval(refreshPendingStats, PENDING_STATS_REFRESH_MS);
    return () => clearInterval(timer);
  }, [refreshPendingStats]);
  return (
    <div className={styles.shell}>
      <div className={styles.dragStrip} data-tauri-drag-region />

      <main ref={contentRef} className={styles.content}>
        {/* 全局横幅只在回填**进行中**出现(任何页面可见可停);空闲时的
            "N 张未索引"提示只属于聊天页(那里才有"对话查不到"的语境)。
            聊天页自带本地横幅,这里避开以免双份。 */}
        {pendingStats?.digestRunning && !location.pathname.startsWith("/chat") && (
          <div className={styles.globalBanner}>
            <BackfillBanner stats={pendingStats} onRefresh={refreshPendingStats} />
          </div>
        )}
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
