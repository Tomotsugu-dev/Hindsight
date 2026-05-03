import { AppIcon } from "../AppIcon/AppIcon";
import styles from "./AppStack.module.css";

interface AppStackProps {
  apps: string[];
  size?: number;
  /** 最多显示几个 */
  max?: number;
  /** 取不到真实图标时画的 fallback 圆点颜色 */
  fallbackColor?: string;
}

/**
 * 多个应用图标的小型重叠堆叠 —— 用于在分类行旁边显示该分类下的代表应用。
 */
export function AppStack({
  apps,
  size = 16,
  max = 3,
  fallbackColor = "#94a3b8",
}: AppStackProps) {
  if (apps.length === 0) return null;
  const shown = apps.slice(0, max);

  return (
    <span className={styles.stack} aria-hidden>
      {shown.map((p, i) => (
        <span
          key={p}
          className={styles.item}
          style={{ zIndex: shown.length - i }}
        >
          <AppIcon processName={p} size={size} fallbackColor={fallbackColor} />
        </span>
      ))}
    </span>
  );
}
