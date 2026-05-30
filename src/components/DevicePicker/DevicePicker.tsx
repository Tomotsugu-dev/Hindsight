import { type CSSProperties } from "react";
import { useTranslation } from "react-i18next";
import { Check, ChevronDown, Layers } from "lucide-react";
import { useDeviceFilter, type Device } from "../../state/deviceFilter";
import { resolveCategoryIcon } from "../../config/categoryIcons";
import { useMouseGlow } from "../../hooks/useMouseGlow";
import { usePicker } from "../../hooks/usePicker";
import styles from "./DevicePicker.module.css";

export function DevicePicker() {
  const { t } = useTranslation();
  const { devices, selected, setSelected } = useDeviceFilter();
  const { open, wrapRef, toggle, close } = usePicker();
  const { ref: triggerRef } = useMouseGlow<HTMLButtonElement>();

  const showAllOption = devices.length >= 2;
  const currentDevice =
    selected === "all" ? null : devices.find((d) => d.id === selected) ?? null;
  const currentLabel =
    selected === "all"
      ? t("components.devicePicker.all")
      : currentDevice?.name ?? t("components.devicePicker.thisDevice");

  return (
    <div className={styles.wrap} ref={wrapRef}>
      <button
        ref={triggerRef}
        type="button"
        className={`${styles.trigger} ${open ? styles.triggerOpen : ""} glow`}
        onClick={toggle}
        aria-haspopup="menu"
        aria-expanded={open}
      >
        <DeviceTile device={currentDevice} all={selected === "all"} />
        <span className={styles.labelStack}>
          <span className={styles.label}>{currentLabel}</span>
        </span>
        <ChevronDown
          size={12}
          strokeWidth={2}
          className={`${styles.chev} ${open ? styles.chevOpen : ""}`}
        />
      </button>

      {open && (
        <div className={styles.menu}>
          {showAllOption && (
            <MenuItem
              label={t("components.devicePicker.all")}
              tile={<DeviceTile device={null} all />}
              checked={selected === "all"}
              onClick={() => {
                setSelected("all");
                close();
              }}
            />
          )}
          {showAllOption && <div className={styles.sep} />}
          {devices.map((d) => (
            <MenuItem
              key={d.id}
              label={d.name}
              hint={d.current ? t("components.devicePicker.thisDevice") : undefined}
              tile={<DeviceTile device={d} all={false} />}
              checked={selected === d.id}
              onClick={() => {
                setSelected(d.id);
                close();
              }}
            />
          ))}
        </div>
      )}
    </div>
  );
}

function DeviceTile({
  device,
  all,
}: {
  device: Device | null;
  all: boolean;
}) {
  if (all || !device) {
    return (
      <span className={`${styles.tile} ${styles.tileMuted}`}>
        <Layers size={12} strokeWidth={2} />
      </span>
    );
  }
  const Icon = resolveCategoryIcon(device.icon);
  const style = { "--tile-color": device.color } as CSSProperties;
  return (
    <span className={styles.tile} style={style}>
      <Icon size={12} strokeWidth={2} />
    </span>
  );
}

interface MenuItemProps {
  label: string;
  hint?: string;
  tile: React.ReactNode;
  checked: boolean;
  onClick: () => void;
}

function MenuItem({ label, hint, tile, checked, onClick }: MenuItemProps) {
  return (
    <button
      type="button"
      className={`${styles.item} ${checked ? styles.itemChecked : ""}`}
      onClick={onClick}
    >
      {tile}
      <span className={styles.itemLabel}>
        {label}
        {hint && <span className={styles.itemHint}> · {hint}</span>}
      </span>
      {checked && <Check size={13} strokeWidth={2.25} className={styles.itemCheck} />}
    </button>
  );
}
