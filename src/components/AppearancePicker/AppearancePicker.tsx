import { useEffect, useRef, type CSSProperties } from "react";
import { useTranslation } from "react-i18next";
import { CATEGORY_ICONS, CATEGORY_PALETTE, ICON_NAMES } from "../../config/categoryIcons";
import styles from "./AppearancePicker.module.css";

interface AppearancePickerProps {
  color: string;
  icon: string;
  onColorChange: (color: string) => void;
  onIconChange: (icon: string) => void;
  onDismiss: () => void;
}

export function AppearancePicker({
  color,
  icon,
  onColorChange,
  onIconChange,
  onDismiss,
}: AppearancePickerProps) {
  const { t } = useTranslation();
  const ref = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const onDown = (e: MouseEvent) => {
      if (!ref.current) return;
      if (!ref.current.contains(e.target as Node)) onDismiss();
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onDismiss();
    };
    document.addEventListener("mousedown", onDown);
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("mousedown", onDown);
      document.removeEventListener("keydown", onKey);
    };
  }, [onDismiss]);

  const styleVar = { "--cat-color": color } as CSSProperties;

  return (
    // 下面的 onMouseDown 不是交互入口,只是把事件截在面板内(理由见 div 上的
    // 注释)。面板本身没有可聚焦语义,加 role/键盘处理反而会误导辅助技术。
    // eslint-disable-next-line jsx-a11y/no-static-element-interactions
    <div
      ref={ref}
      className={styles.popover}
      style={styleVar}
      // 面板是独立浮层,但 DOM 上是分类行 / 大类卡片的后代,而那两层都用
      // mousedown 启动拖拽、只放行 button|input|[role=button]——面板的空白处、
      // 「颜色」「图标」标签、图标网格的滚动条都不在放行名单里,按住就等于
      // 按住了卡片,整组被拖起来。这里把内部 mousedown 截住。
      // 用 React 合成事件的 stopPropagation:外部点击关闭走的是原生 document
      // 监听(在 React root 之外),不受影响,仍能正常关闭。
      onMouseDown={(e) => e.stopPropagation()}
    >
      <div className={styles.section}>
        <span className={styles.label}>{t("components.appearancePicker.color")}</span>
        <div className={styles.colorRow}>
          {CATEGORY_PALETTE.map((c) => (
            <button
              key={c}
              type="button"
              className={`${styles.swatch} ${
                c.toLowerCase() === color.toLowerCase() ? styles.swatchActive : ""
              }`}
              style={{ background: c }}
              onClick={() => onColorChange(c)}
              aria-label={c}
            />
          ))}
        </div>
      </div>

      <div className={styles.section}>
        <span className={styles.label}>{t("components.appearancePicker.icon")}</span>
        <div className={styles.iconGrid}>
          {ICON_NAMES.map((name) => {
            const Icon = CATEGORY_ICONS[name];
            const active = name === icon;
            return (
              <button
                key={name}
                type="button"
                className={`${styles.iconBtn} ${active ? styles.iconBtnActive : ""}`}
                onClick={() => onIconChange(name)}
                aria-label={name}
                title={name}
              >
                <Icon size={16} strokeWidth={1.85} />
              </button>
            );
          })}
        </div>
      </div>
    </div>
  );
}
