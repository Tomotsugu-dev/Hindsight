import { useState } from "react";
import { convertFileSrc } from "@tauri-apps/api/core";
import { Globe } from "lucide-react";
import styles from "./SiteIcon.module.css";

/** A missing or unreadable cached file keeps the same globe placeholder as an undownloaded icon. */
export function SiteIcon({ path }: { path?: string }) {
  const [failedPath, setFailedPath] = useState<string | null>(null);
  if (!path || path === failedPath) {
    return <Globe size={14} strokeWidth={2} className={styles.icon} aria-hidden />;
  }
  return (
    <img
      className={styles.icon}
      src={convertFileSrc(path)}
      width={16}
      height={16}
      alt=""
      aria-hidden
      draggable={false}
      onError={() => setFailedPath(path)}
    />
  );
}
