import { useEffect, useRef } from "react";

/**
 * 跟随鼠标的"接近触发"glow：鼠标在 window 上移动时，对每个挂了 ref 的按钮
 * 实时算"鼠标到按钮最近边的距离"，把：
 *   --glow-mx / --glow-my：鼠标相对按钮左上角的坐标（用于 radial-gradient 中心）
 *   --glow-strength      ：0~1 的强度（用于 ::before 的 opacity）
 * 写到元素 style 上。
 *
 * 离按钮 PROXIMITY_RADIUS 像素以内 strength = 1 - dist / RADIUS，鼠标进按钮即 1，越远越淡。
 *
 * 用法：
 *   const { ref } = useMouseGlow<HTMLButtonElement>();
 *   <button ref={ref} className={`${styles.btn} glow`}>...</button>
 */

const PROXIMITY_RADIUS = 180;

const tracked = new Set<HTMLElement>();
let listening = false;

function refresh(e: MouseEvent) {
  for (const el of tracked) {
    const r = el.getBoundingClientRect();
    const mx = e.clientX - r.left;
    const my = e.clientY - r.top;
    // 鼠标到按钮 bbox 最近边的距离（鼠标在按钮内时为 0）
    const dx = Math.max(r.left - e.clientX, e.clientX - r.right, 0);
    const dy = Math.max(r.top - e.clientY, e.clientY - r.bottom, 0);
    const dist = Math.hypot(dx, dy);
    const strength = Math.max(0, 1 - dist / PROXIMITY_RADIUS);
    el.style.setProperty("--glow-mx", `${mx}px`);
    el.style.setProperty("--glow-my", `${my}px`);
    el.style.setProperty("--glow-strength", strength.toFixed(3));
  }
}

function fadeOut() {
  for (const el of tracked) {
    el.style.setProperty("--glow-strength", "0");
  }
}

function ensureListening() {
  if (listening) return;
  listening = true;
  window.addEventListener("mousemove", refresh, { passive: true });
  document.addEventListener("mouseleave", fadeOut);
  window.addEventListener("blur", fadeOut);
}

export function useMouseGlow<T extends HTMLElement = HTMLElement>() {
  const ref = useRef<T>(null);
  useEffect(() => {
    const el = ref.current;
    if (!el) return;
    tracked.add(el);
    ensureListening();
    return () => {
      tracked.delete(el);
    };
  }, []);
  return { ref };
}
