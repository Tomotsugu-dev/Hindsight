import styles from "./SegmentedControl.module.css";

export interface SegmentedOption<T extends string> {
  value: T;
  label: string;
}

interface SegmentedControlProps<T extends string> {
  value: T;
  options: SegmentedOption<T>[];
  onChange: (next: T) => void;
  disabled?: boolean;
  /** 无障碍分组名（读屏播报这组按钮是干什么的） */
  ariaLabel?: string;
}

/**
 * 分段选择器：选项少（2-4 个）且值域固定时，比 [SimplePicker] 更直观
 * ——所有候选一眼可见、一次点击切换，不用先展开。
 *
 * 视觉沿用 TabNav 的胶囊语言（凹槽底 + 选中项白底浮起），尺寸对齐 SimplePicker
 * 的 trigger，放进 Row 右侧不会比下拉重。选中态不做滑动动画：这里是设置项，
 * 一次一个决定，弹簧位移反而喧宾夺主。
 */
export function SegmentedControl<T extends string>({
  value,
  options,
  onChange,
  disabled,
  ariaLabel,
}: SegmentedControlProps<T>) {
  return (
    <div className={styles.wrap} role="group" aria-label={ariaLabel}>
      {options.map((opt) => {
        const active = opt.value === value;
        return (
          <button
            key={opt.value}
            type="button"
            className={`${styles.seg} ${active ? styles.segActive : ""}`}
            onClick={() => !disabled && !active && onChange(opt.value)}
            disabled={disabled}
            aria-pressed={active}
          >
            {opt.label}
          </button>
        );
      })}
    </div>
  );
}
