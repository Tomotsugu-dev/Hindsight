import { useEffect, useRef, useState } from "react";
import { Check, ChevronDown, Layers, Monitor } from "lucide-react";
import { useDeviceFilter } from "../../state/deviceFilter";
import styles from "./DevicePicker.module.css";

export function DevicePicker() {
  const { devices, selected, setSelected } = useDeviceFilter();
  const [open, setOpen] = useState(false);
  const wrapRef = useRef<HTMLDivElement>(null);

  const showAllOption = devices.length >= 2;
  const currentLabel =
    selected === "all"
      ? "所有设备"
      : devices.find((d) => d.id === selected)?.name ?? "本机";

  // 点外面关闭
  useEffect(() => {
    if (!open) return;
    const onClick = (e: MouseEvent) => {
      if (wrapRef.current && !wrapRef.current.contains(e.target as Node)) {
        setOpen(false);
      }
    };
    document.addEventListener("mousedown", onClick);
    return () => document.removeEventListener("mousedown", onClick);
  }, [open]);

  return (
    <div className={styles.wrap} ref={wrapRef}>
      <button
        type="button"
        className={`${styles.trigger} ${open ? styles.triggerOpen : ""}`}
        onClick={() => setOpen((v) => !v)}
        aria-haspopup="listbox"
        aria-expanded={open}
      >
        <span className={styles.label}>{currentLabel}</span>
        <ChevronDown
          size={12}
          strokeWidth={2}
          className={`${styles.chev} ${open ? styles.chevOpen : ""}`}
        />
      </button>

      {open && (
        <div className={styles.menu} role="listbox">
          {showAllOption && (
            <MenuItem
              label="所有设备"
              icon={<Layers size={13} strokeWidth={1.85} />}
              checked={selected === "all"}
              onClick={() => {
                setSelected("all");
                setOpen(false);
              }}
            />
          )}
          {showAllOption && <div className={styles.sep} />}
          {devices.map((d) => (
            <MenuItem
              key={d.id}
              label={d.name}
              hint={d.current ? "本机" : undefined}
              icon={<Monitor size={13} strokeWidth={1.85} />}
              checked={selected === d.id}
              onClick={() => {
                setSelected(d.id);
                setOpen(false);
              }}
            />
          ))}
        </div>
      )}
    </div>
  );
}

interface MenuItemProps {
  label: string;
  hint?: string;
  icon?: React.ReactNode;
  checked: boolean;
  onClick: () => void;
}

function MenuItem({ label, hint, icon, checked, onClick }: MenuItemProps) {
  return (
    <button
      type="button"
      className={`${styles.item} ${checked ? styles.itemChecked : ""}`}
      onClick={onClick}
      role="option"
      aria-selected={checked}
    >
      <span className={styles.itemIcon}>{icon}</span>
      <span className={styles.itemLabel}>
        {label}
        {hint && <span className={styles.itemHint}> · {hint}</span>}
      </span>
      {checked && <Check size={13} strokeWidth={2.25} className={styles.itemCheck} />}
    </button>
  );
}
