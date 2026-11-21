import { Check, ChevronDown, type LucideIcon } from "lucide-react";
import { usePicker } from "../../hooks/usePicker";
import styles from "./InlinePicker.module.css";

export interface InlinePickerOption {
  value: string;
  label: string;
  icon?: LucideIcon;
  /** 右边的灰字 */
  detail?: string;
  /** 云端：数据会发出设备，图标用琥珀色 */
  cloud?: boolean;
  /** 悬停提示，比如完整的文件名 */
  title?: string;
}

interface InlinePickerProps {
  /** 选中的值；一项都对不上时是 null */
  value: string | null;
  options: InlinePickerOption[];
  onChange: (value: string) => void;
  /** 按钮上的字，写短一点，全名在菜单里 */
  label: string;
  icon: LucideIcon;
  cloud?: boolean;
  /** 菜单顶上的小标题，也是按钮的悬停提示 */
  title: string;
  /** 菜单底部的灰字 */
  note?: string;
  disabled?: boolean;
  /** 每次打开前调用，比如重新读一遍列表 */
  onOpen?: () => void;
}

/**
 * 放在一行字里的小下拉框，照 VS Code 对话框底部的选择器：平时没有边框，只有图标、短名字和箭头；
 * 菜单可以比按钮宽，名字写全。
 *
 * The button carries `data-part="select"` and `data-size="inline"`, and the menu
 * `data-part="menu"`, for themes.
 */
export function InlinePicker({
  value,
  options,
  onChange,
  label,
  icon: Icon,
  cloud,
  title,
  note,
  disabled,
  onOpen,
}: InlinePickerProps) {
  const { open, wrapRef, toggle, close } = usePicker();

  return (
    <div className={styles.wrap} ref={wrapRef}>
      <button
        type="button"
        className={`${styles.trigger} ${open ? styles.triggerOpen : ""}`}
        onClick={() => {
          if (!open) onOpen?.();
          toggle();
        }}
        disabled={disabled}
        aria-haspopup="menu"
        aria-expanded={open}
        title={title}
        aria-label={`${title}: ${label}`}
        data-part="select"
        data-size="inline"
      >
        <Icon className={`${styles.icon} ${cloud ? styles.cloud : ""}`} aria-hidden />
        <span className={styles.label}>{label}</span>
        <ChevronDown className={styles.chev} aria-hidden />
      </button>

      {open ? (
        <div className={styles.menu} role="menu" aria-label={title} data-part="menu">
          <p className={styles.heading}>{title}</p>
          {options.map((o) => {
            const selected = o.value === value;
            const ItemIcon = o.icon;
            return (
              <button
                key={o.value}
                type="button"
                role="menuitemradio"
                aria-checked={selected}
                className={`${styles.item} ${selected ? styles.itemChecked : ""}`}
                title={o.title}
                onClick={() => {
                  close();
                  if (!selected) onChange(o.value);
                }}
              >
                {ItemIcon ? (
                  <ItemIcon
                    className={`${styles.itemIcon} ${o.cloud ? styles.cloud : ""}`}
                    aria-hidden
                  />
                ) : null}
                <span className={styles.itemLabel}>{o.label}</span>
                {o.detail ? <span className={styles.itemDetail}>{o.detail}</span> : null}
                <Check
                  strokeWidth={2.25}
                  className={`${styles.itemCheck} ${selected ? "" : styles.hidden}`}
                  aria-hidden
                />
              </button>
            );
          })}
          {note ? <p className={styles.note}>{note}</p> : null}
        </div>
      ) : null}
    </div>
  );
}
