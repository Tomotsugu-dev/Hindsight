import { useEffect, useRef, useState } from "react";
import { Outlet } from "react-router-dom";
import { Sidebar } from "../components/Sidebar/Sidebar";
import { WindowControls } from "../components/WindowControls/WindowControls";
import styles from "./AppLayout.module.css";

const RAIL_WIDTH = 56;
const FULL_WIDTH = 232;
const DRAG_STRIP_HEIGHT = 36;
const HOVER_INTENT_MS = 150;

export function AppLayout() {
  const [expanded, setExpanded] = useState(false);
  const expandTimerRef = useRef<number | null>(null);

  useEffect(() => {
    const cancel = () => {
      if (expandTimerRef.current) {
        window.clearTimeout(expandTimerRef.current);
        expandTimerRef.current = null;
      }
    };

    const onMove = (e: MouseEvent) => {
      const inRailZone = e.clientX <= RAIL_WIDTH && e.clientY > DRAG_STRIP_HEIGHT;
      const insideExpanded = e.clientX <= FULL_WIDTH;

      if (expanded) {
        if (!insideExpanded) {
          cancel();
          setExpanded(false);
        }
      } else {
        if (inRailZone) {
          if (expandTimerRef.current == null) {
            expandTimerRef.current = window.setTimeout(() => {
              expandTimerRef.current = null;
              setExpanded(true);
            }, HOVER_INTENT_MS);
          }
        } else {
          cancel();
        }
      }
    };

    const onLeave = () => {
      cancel();
      setExpanded(false);
    };

    window.addEventListener("mousemove", onMove);
    document.addEventListener("mouseleave", onLeave);
    return () => {
      window.removeEventListener("mousemove", onMove);
      document.removeEventListener("mouseleave", onLeave);
      cancel();
    };
  }, [expanded]);

  return (
    <div className={`${styles.shell} ${expanded ? styles.expanded : ""}`}>
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
