import { useEffect, useState } from "react";
import { api } from "../../api/hindsight";
import styles from "./AppIcon.module.css";

const cache = new Map<string, string | null>();
const inflight = new Map<string, Promise<string | null>>();

interface AppIconProps {
  processName: string;
  fallbackColor: string;
  size?: number;
}

export function AppIcon({ processName, fallbackColor, size = 18 }: AppIconProps) {
  const [src, setSrc] = useState<string | null | undefined>(() =>
    cache.get(processName) ?? undefined,
  );

  useEffect(() => {
    if (src !== undefined) return;

    if (cache.has(processName)) {
      setSrc(cache.get(processName) ?? null);
      return;
    }

    let cancelled = false;
    const existing = inflight.get(processName);
    const p =
      existing ??
      api
        .getAppIcon(processName)
        .then((data) => {
          cache.set(processName, data ?? null);
          inflight.delete(processName);
          return data ?? null;
        })
        .catch(() => {
          cache.set(processName, null);
          inflight.delete(processName);
          return null;
        });
    if (!existing) inflight.set(processName, p);

    p.then((data) => {
      if (!cancelled) setSrc(data);
    });

    return () => {
      cancelled = true;
    };
  }, [processName, src]);

  if (src) {
    return (
      <img
        className={styles.icon}
        src={src}
        alt={processName}
        width={size}
        height={size}
      />
    );
  }

  return (
    <span
      className={styles.dot}
      style={{ background: fallbackColor, width: size, height: size }}
      aria-hidden
    />
  );
}
