// How each component looks: small in the closed island, bigger in the open one.
import { invoke } from "@tauri-apps/api/core";
import { readable } from "./color";
import { islandIcon } from "./icons";
import { formatSeconds, secondsLeft, shareLeft } from "./time";
import type { Component, Text } from "./types";
import styles from "./Island.module.css";

const tint = (color: string | null) => (color ? readable(color) : undefined);

function Icon({ name, color, size }: { name: string; color: string | null; size: number }) {
  const Shape = islandIcon(name);
  return <Shape size={size} color={tint(color) ?? "#fff"} strokeWidth={2.25} />;
}

/** 计时器的环两头之间至少空多少：剩的时间是满的也看得出是个环 */
const TIMER_GAP = 2;

type RingProps = { share: number; size: number; color: string | null; gap?: boolean };

/** `gap`：两头之间留一条缝，计时器用。`ring` 组件不留，它的 1 是做完了，画满圈 */
function Ring({ share, size, color, gap = false }: RingProps) {
  const stroke = Math.max(2, size / 8);
  const r = (size - stroke) / 2;
  const length = 2 * Math.PI * r;
  // 两头的圆角各伸出半个线宽，所以还要多让一个线宽。按比例缩短，环一开始就跟着时间走
  const most = gap ? Math.max(0, (length - stroke - TIMER_GAP) / length) : 1;
  return (
    <svg className={styles.ring} width={size} height={size} viewBox={`0 0 ${size} ${size}`}>
      <circle cx={size / 2} cy={size / 2} r={r} className={styles.ringTrack} strokeWidth={stroke} />
      <circle
        cx={size / 2}
        cy={size / 2}
        r={r}
        fill="none"
        stroke={tint(color) ?? "#fff"}
        strokeWidth={stroke}
        strokeLinecap="round"
        strokeDasharray={length}
        strokeDashoffset={length * (1 - share * most)}
      />
    </svg>
  );
}

export function TextView({ text }: { text: Text }) {
  return (
    <span className={text.size === "large" ? styles.large : styles.small}>
      {text.text.map((run, i) => (
        <span
          key={i}
          className={[run.dim && styles.dim, run.bold && styles.bold].filter(Boolean).join(" ")}
          style={{ color: tint(run.color) }}
        >
          {run.text}
        </span>
      ))}
      {text.sub ? <span className={styles.sub}>{text.sub}</span> : null}
    </span>
  );
}

/** A component on one side of the closed island. Cells and buttons never go there. */
export function Side({ component, now }: { component: Component; now: number }) {
  switch (component.type) {
    case "icon":
      return <Icon name={component.name} color={component.color} size={14} />;
    case "text":
      return <TextView text={component} />;
    case "timer":
      return (
        <span className={styles.inline}>
          <Ring share={shareLeft(component, now)} size={14} color={component.color} gap />
          <span className={styles.time}>{formatSeconds(secondsLeft(component, now))}</span>
        </span>
      );
    case "ring":
      return (
        <span className={styles.inline}>
          <Ring share={component.value} size={14} color={component.color} />
          {component.text ? <span className={styles.time}>{component.text}</span> : null}
        </span>
      );
    default:
      return null;
  }
}

/** A component in the open island. */
export function Block({ component, now }: { component: Component; now: number }) {
  switch (component.type) {
    case "icon":
      return <Icon name={component.name} color={component.color} size={22} />;
    case "text":
      return <TextView text={component} />;
    case "timer":
      return (
        <span className={styles.inline}>
          <Ring share={shareLeft(component, now)} size={20} color={component.color} gap />
          <span className={`${styles.large} ${styles.time}`}>
            {formatSeconds(secondsLeft(component, now))}
          </span>
        </span>
      );
    case "ring":
      return (
        <span className={styles.bigRing}>
          <Ring share={component.value} size={52} color={component.color} />
          {component.text ? <span className={styles.ringText}>{component.text}</span> : null}
        </span>
      );
    case "cells":
      return (
        <span
          className={styles.cells}
          style={{ gridTemplateColumns: `repeat(${component.columns}, minmax(0, 10px))` }}
        >
          {component.colors.map((color, i) => (
            <span
              key={i}
              className={i === component.current ? `${styles.cell} ${styles.current}` : styles.cell}
              style={color ? { background: readable(color) } : undefined}
            />
          ))}
        </span>
      );
    case "button":
      return (
        <button
          type="button"
          className={component.text ? styles.button : `${styles.button} ${styles.iconOnly}`}
          onClick={(e) => {
            // A button is not a click on the island: it does not open the page
            e.stopPropagation();
            void invoke("island_clicked", { button: component.id });
          }}
        >
          <Icon name={component.icon} color={null} size={16} />
          {component.text ? <span>{component.text}</span> : null}
        </button>
      );
  }
}
